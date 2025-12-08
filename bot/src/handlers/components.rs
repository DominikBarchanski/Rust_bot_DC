use crate::db::repo;
use crate::handlers::{pool_from_ctx, redis_from_ctx};
use crate::queue;
use crate::ui::{embeds, menus};
use crate::utils::{from_user_id, parse_component_id,mention_user,user_name_best,notify_raid_now,dm_user,ORGANISER_ROLE_NAME};
use dashmap::DashMap;
use std::collections::{HashMap,HashSet};
use once_cell::sync::Lazy;
use chrono::{DateTime,Duration as DD ,Utc};
use serenity::all::*;
use serenity::builder::{
    CreateInteractionResponse,
    CreateInteractionResponseMessage,
    EditInteractionResponse,
};
use chrono_tz::Europe::Warsaw;
use serenity::builder::{EditMessage, CreateMessage};
use tokio::time::{sleep, Duration};
use uuid::Uuid;
use std::env;

#[derive(Default, Clone)]
struct JoinSelection {
    class: Option<String>,
    sp: Option<String>,
    main: bool, // true -> main, false -> alt (click source)
}

static JOIN_STATE: Lazy<DashMap<(u64, Uuid), JoinSelection>> = Lazy::new(DashMap::new);
static OWNER_CHANGE: Lazy<DashMap<(u64, Uuid), u64>> = Lazy::new(DashMap::new);
pub async fn handle_component(ctx: &Context, it: &ComponentInteraction) -> anyhow::Result<()> {
    let Some((kind, which, raid_id)) = parse_component_id(&it.data.custom_id) else { return Ok(()); };

    match (kind.as_str(), which.as_str()) {
        ("j", "m") => show_join_menu(ctx, it, raid_id, true).await?,
        ("j", "a") => show_join_menu(ctx, it, raid_id, false).await?,
        ("pc", "") => save_pick(ctx, it, raid_id, true).await?,
        ("ps", "") => save_pick(ctx, it, raid_id, false).await?,
        ("ok", "") => confirm_join(ctx, it, raid_id).await?,
        ("l",  "") => leave_all(ctx, it, raid_id).await?,
        ("la", "") => leave_alts(ctx, it, raid_id).await?,
        ("mg", "") => owner_manage(ctx, it, raid_id).await?,
        ("pr", "") => owner_promote(ctx, it, raid_id).await?,
        ("mr", "") => owner_move_to_reserve(ctx, it, raid_id).await?,
        ("kk", "") => owner_kick(ctx, it, raid_id).await?,
        ("cx", "") => owner_cancel(ctx, it, raid_id).await?,
        ("cl", "") => close_ephemeral(ctx,it).await?,
        ("not", "") => notify_raid_now(ctx,raid_id).await?,
        ("cho", "") => owner_change_start(ctx, it, raid_id).await?,   // show picker
        ("chp", "") => owner_change_pick(ctx, it, raid_id).await?,    // store pick
        ("chc", "") => owner_change_confirm(ctx, it, raid_id).await?, // confirm + transfer
        _ => {}
    }

    Ok(())
}

async fn show_join_menu(
    ctx: &Context,
    it: &ComponentInteraction,
    raid_id: Uuid,
    main: bool,
) -> anyhow::Result<()> {
    JOIN_STATE.insert((it.user.id.get(), raid_id), JoinSelection { class: None, sp: None, main });

    let content = "Pick your class and SP:\nSelected: **—** / **—**";

    it.create_response(
        &ctx.http,
        CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content(content)
                .ephemeral(true)
                .components(vec![
                    menus::class_menu_row_selected(raid_id, None),
                    menus::sp_menu_row_selected(raid_id, None, None),
                    menus::confirm_row(raid_id, main),
                ]),
        ),
    )
        .await?;
    Ok(())
}


async fn save_pick(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid, is_class: bool) -> anyhow::Result<()> {
    if let ComponentInteractionDataKind::StringSelect { values } = &it.data.kind {
        if let Some(v) = values.first() {
            let key = (it.user.id.get(), raid_id);
            let mut entry = JOIN_STATE.entry(key).or_default();
            if is_class {
                // jeżeli zmienił klasę, wyczyść SP jeśli nie pasuje do nowej listy
                entry.class = Some(v.clone());
                if let Some(sp) = &entry.sp {
                    let msw_only = matches!(v.as_str(), "MSW" | "msw");
                    let sp_num_ok = |s: &str| {
                        let s = s.trim().to_ascii_uppercase();
                        if !s.starts_with("SP") { return false; }
                        let n: i32 = s[2..].parse().unwrap_or(0);
                        if msw_only { [1,2,3,4,9,10,11].contains(&n) } else { (1..=11).contains(&n) }
                    };
                    if !sp_num_ok(sp) {
                        entry.sp = None;
                    }
                }
            } else {
                entry.sp = Some(v.clone());
            }
        }
    }

    let state = JOIN_STATE.get(&(it.user.id.get(), raid_id)).map(|r| r.clone()).unwrap_or_default();
    let class_txt = state.class.as_deref().unwrap_or("—");
    let sp_txt = state.sp.as_deref().unwrap_or("—");
    let content = format!("Pick your class and SP:\nSelected: **{}** / **{}**", class_txt, sp_txt);

    it.create_response(
        &ctx.http,
        CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new()
                .content(content)
                .components(vec![
                    menus::class_menu_row_selected(raid_id, state.class.as_deref()),
                    menus::sp_menu_row_selected(raid_id, state.class.as_deref(), state.sp.as_deref()),
                    menus::confirm_row(raid_id, state.main),
                ]),
        ),
    )
        .await?;
    Ok(())
}


async fn confirm_join(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let key = (it.user.id.get(), raid_id);
    let Some(sel) = JOIN_STATE.get(&key).map(|r| r.value().clone()) else { return Ok(()); };

    // 1) Szybki ACK (jedyna create_response w tej funkcji)
    let _ = it.create_response(
        &ctx.http,
        CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new()
                .content("⏳ Processing your request...")
                .components(Vec::new()),
        ),
    ).await;

    // 2) Dalej już tylko edycje tej odpowiedzi
    if sel.class.is_none() || sel.sp.is_none() {
        it.edit_response(&ctx.http, EditInteractionResponse::new()
            .content("Please choose both class and SP.")
        ).await?;
        sleep(Duration::from_secs(5)).await;
        let _ = it.delete_response(&ctx.http).await;
        return Ok(());
    }

    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if !raid.is_active {
        it.edit_response(&ctx.http, EditInteractionResponse::new()
            .content("This raid has been cancelled.")
        ).await?;
        return Ok(());
    }

    // === WYMÓG ról c1-89 lub c90 ===
    let mut allowed_by_crole = false;
    let mut has_c1_89 = false;

    if let Some(gid) = it.guild_id {
        let roles_map = gid.roles(&ctx.http).await?;
        if let Ok(member) = gid.member(&ctx.http, it.user.id).await {
            for rid in &member.roles {
                if let Some(r) = roles_map.get(rid) {
                    let name = r.name.to_ascii_lowercase();
                    if name == "c1-89" || name == "c90" {
                        if name == "c1-89" { has_c1_89 = true; }
                        allowed_by_crole = true;
                        break;
                    }
                }
            }
        }
    }

    if !allowed_by_crole {
        it.edit_response(&ctx.http, EditInteractionResponse::new()
            .content("You need role **c1-89** or **c90**, to join this raid.")
        ).await?;
        return Ok(());
    }

    let tag_suffix = if has_c1_89 { " [-c90]".to_string() } else { String::new() };

    // reserve role always → reserve
    let reserve_role_name = env::var("RESERVE_ROLE_NAME").unwrap_or_else(|_| "reserve".to_string());
    let mut force_reserve = false;
    if let Some(gid) = it.guild_id {
        let roles_map = gid.roles(&ctx.http).await?;
        if let Ok(member) = gid.member(&ctx.http, it.user.id).await {
            for rid in &member.roles {
                if let Some(r) = roles_map.get(rid) {
                    if r.name.eq_ignore_ascii_case(&reserve_role_name) { force_reserve = true; break; }
                }
            }
        }
    }

    let alt_allow_role_name = std::env::var("ALT_ALLOW_ROLE_NAME").unwrap_or_else(|_| "Alt_allow".to_string());
    let mut has_alt_allow_role = false;
    if let Some(gid) = it.guild_id {
        let roles_map = gid.roles(&ctx.http).await?;
        if let Ok(member) = gid.member(&ctx.http, it.user.id).await {
            for rid in &member.roles {
                if let Some(r) = roles_map.get(rid) {
                    if r.name.eq_ignore_ascii_case(&alt_allow_role_name) {
                        has_alt_allow_role = true;
                        break;
                    }
                }
            }
        }
    }

    // Priority: jeśli okno aktywne i brak roli → reserve
    let mut must_reserve = force_reserve;
    if let Some(until) = raid.priority_until {
        if chrono::Utc::now() < until {
                let has_priority = if let Some(gid) = it.guild_id {
                    let member = gid.member(&ctx.http, it.user.id).await?;
                    let user_roles: HashSet<u64> = member.roles.iter().map(|r| r.get()).collect();
                    match &raid.priority_role_id {
                        Some(ids) if !ids.is_empty() => ids.iter().any(|id| user_roles.contains(&(*id as u64))),
                        _ => false,
                    }
                } else { false };
            if !has_priority { must_reserve = true; }
        }
    }

    // Pojemność
    let mains_cnt = repo::count_mains(&pool, raid_id).await? as i32;
    let free_main = (raid.max_players - mains_cnt).max(0);

    // Pola
    let joined_as = format!("{} / {}", sel.class.clone().unwrap(), sel.sp.clone().unwrap());
    let is_alt_join = !sel.main;
    let requester_id = from_user_id(it.user.id);

    if is_alt_join && !repo::user_has_main(&pool, raid_id, requester_id).await? {
        it.edit_response(&ctx.http, EditInteractionResponse::new()
            .content("You have to sign as a main before add alt.")
        ).await?;
        return Ok(());
    }

    if is_alt_join {
        if !has_alt_allow_role {
            it.edit_response(&ctx.http, EditInteractionResponse::new()
                .content(&format!("You are not allowed to sign in an alt. Missing role: {}", alt_allow_role_name))
            ).await?;
            return Ok(());
        }
        if !raid.allow_alts {
            it.edit_response(&ctx.http, EditInteractionResponse::new()
                .content("Alts are disabled for this raid.")
            ).await?;
            return Ok(());
        }

        let user_alt_count = repo::alt_count_for_user(&pool, raid_id, from_user_id(it.user.id)).await? as i32;
        if user_alt_count >= raid.max_alts {
            it.edit_response(&ctx.http, EditInteractionResponse::new()
                .content("You've reached your alt limit for this raid.")
            ).await?;
            return Ok(());
        }
    }

    // Publish to Redis queue and wait briefly for ACK
    let can_be_main = free_main > 0 && !must_reserve;
    let redis = redis_from_ctx(ctx).await?;
    let ev = queue::RaidEvent::Join {
        raid_id,
        guild_id: raid.guild_id,
        user_id: from_user_id(it.user.id),
        joined_as,
        main_now: can_be_main,
        tag_suffix: tag_suffix.clone(),
        is_alt: is_alt_join,
    };
    let corr = queue::publish(&redis, &ev).await?;
    let _ack = queue::wait_for_ack(&redis, &corr, 900).await?; // best-effort


    // Odśwież wiadomość
    let raid = repo::get_raid(&pool, raid_id).await?; // re-read after ack
    let parts = repo::list_participants(&pool, raid_id).await?;
    let embed = embeds::render_raid_embed(ctx, raid.guild_id as u64, &raid, &parts);
    ChannelId::new(raid.channel_id as u64)
        .edit_message(&ctx.http, raid.message_id as u64,
                      EditMessage::new()
                          .embed(embed)
                          .components(vec![menus::main_buttons_row(raid_id)])
        ).await?;

    // Finalny komunikat do użytkownika – edycja tej samej odpowiedzi
    it.edit_response(&ctx.http, EditInteractionResponse::new()
        .content("You're signed in! ✅")
    ).await?;
    sleep(Duration::from_secs(5)).await;
    let _ = it.delete_response(&ctx.http).await;

    // also try to refresh guild list immediately (force, non-debounced)
    let _ = crate::commands::raid::force_refresh_guild_raid_list(ctx, raid.guild_id as u64).await;

    JOIN_STATE.remove(&key);
    Ok(())
}

async fn leave_all(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if !raid.is_active {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("This raid has been cancelled.")
                .ephemeral(true)
        )).await?;
        return Ok(());
    }

    // Publish to queue
    let redis = redis_from_ctx(ctx).await?;
    let user_id = from_user_id(it.user.id);
    let ev = queue::RaidEvent::LeaveAll { raid_id, guild_id: raid.guild_id, user_id };
    let corr = queue::publish(&redis, &ev).await?;
    let ack = queue::wait_for_ack(&redis, &corr, 900).await?;

    // Re-read raid and refresh UI; notify if something actually removed
    let raid = repo::get_raid(&pool, raid_id).await?;
    let time_left = human_time_left(raid.scheduled_for);
    let removed_any = ack.as_ref().map(|a| a.removed_main.unwrap_or(0) + a.removed_alts.unwrap_or(0) > 0).unwrap_or(true);
    if removed_any {
        let owner_id = UserId::new(raid.owner_id as u64);
        let dm = owner_id.create_dm_channel(&ctx.http).await?;
        dm.id
            .send_message(
                &ctx.http,
                CreateMessage::new().content(format!(
                    "Heads up: user {user} signed off from raid \"{name}\".\nChannel: <#{chan}>\nStarts in: {left}",
                    user = mention_user(user_id),
                    name = raid.raid_name,
                    chan = raid.channel_id as u64,
                    left = time_left
                )),
            )
            .await?;

        ChannelId::new(raid.channel_id as u64)
            .send_message(
                &ctx.http,
                CreateMessage::new().content(format!(
                    "<@{user}> signed off. Raid starts in {left}.",
                    user = user_id,
                    left = time_left
                )),
            )
            .await?;
        let _ = refresh_message(ctx, it, raid_id, "You have been signed out.").await;
    } else {
        let _ = refresh_message(ctx, it, raid_id, "You were already signed out.").await;
    }
    Ok(())
}

async fn leave_alts(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if !raid.is_active {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("This raid has been cancelled. ")
                .ephemeral(true)
        )).await?;
        return Ok(());
    }
    let redis = redis_from_ctx(ctx).await?;
    let ev = queue::RaidEvent::LeaveAlts { raid_id, guild_id: raid.guild_id, user_id: from_user_id(it.user.id) };
    let corr = queue::publish(&redis, &ev).await?;
    let _ack = queue::wait_for_ack(&redis, &corr, 900).await?;
    refresh_message(ctx, it, raid_id, "Removed your alts.").await
}

async fn refresh_message(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid, tip: &str) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    let participants = repo::list_participants(&pool, raid_id).await?;
    let embed = embeds::render_raid_embed(ctx, raid.guild_id as u64, &raid, &participants);
    ChannelId::new(raid.channel_id as u64)
        .edit_message(&ctx.http, raid.message_id as u64,
                      EditMessage::new().embed(embed).components(vec![menus::main_buttons_row(raid_id)]))
        .await?;
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content(tip).ephemeral(true)
        )).await?;
    // refresh guild list immediately (force, non-debounced)
    let _ = crate::commands::raid::force_refresh_guild_raid_list(ctx, raid.guild_id as u64).await;
    Ok(())
}

async fn owner_manage(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if raid.owner_id != it.user.id.get() as i64 {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Only the raid owner can manage.").ephemeral(true)
        )).await?;
        return Ok(());
    }
    if !raid.is_active {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("This raid has been cancelled. Managing is no longer available.")
                .ephemeral(true)
        )).await?;
        return Ok(());
    }
    let gid_u64 = raid.guild_id as u64;
    let mut name_map: HashMap<i64, String> = HashMap::new();

    let parts = repo::list_participants(&pool, raid_id).await?;

    for p in &parts {
        if !name_map.contains_key(&p.user_id) {
            let name = user_name_best(ctx, Some(gid_u64), p.user_id).await;
            name_map.insert(p.user_id, name);
        }
    }


    // reserves-only options for Promote
    let mut promote_opts: Vec<(String, String)> = parts
        .iter()
        .filter(|p| !p.is_main)
        .map(|p| {
            let name = name_map.get(&p.user_id)
                .cloned()
                .unwrap_or_else(|| format!("user {}", p.user_id));
            (
                format!("{} {}{}", p.joined_as, name, if p.is_alt { " (ALT)" } else { "" }),
                p.id.to_string(),
            )
        })
        .collect();

    let mut promote_to_reserve_opts: Vec<(String, String)> = parts
        .iter()
        .filter(|p| p.is_main)
        .map(|p| {
            let name = name_map.get(&p.user_id)
                .cloned()
                .unwrap_or_else(|| format!("user {}", p.user_id));
            (
                format!("{} {}{}", p.joined_as, name, if p.is_alt { " (ALT)" } else { "" }),
                p.id.to_string(),
            )
        })
        .collect();

    let mut kick_opts: Vec<(String, String)> = parts
        .iter()
        .map(|p| {
            let name = name_map.get(&p.user_id)
                .cloned()
                .unwrap_or_else(|| format!("user {}", p.user_id));
            (
                format!(
                    "{} {}{}{}",
                    p.joined_as,
                    name,
                    if p.is_main { " [MAIN]" } else { " [RES]" },
                    if p.is_alt { " (ALT)" } else { "" }
                ),
                p.id.to_string(),
            )
        })
        .collect();

    if kick_opts.is_empty() {
        kick_opts.push(("No participants".into(), "none".into()));
    }

    if promote_to_reserve_opts.is_empty() {
        promote_to_reserve_opts.push(("No reserves".into(), "none".into()));
    }


    if promote_opts.is_empty() {
        promote_opts.push(("No reserves".into(), "none".into()));
    }

    // Any participants for Kick


    it.create_response(&ctx.http, CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content("Owner controls")
            .ephemeral(true)
            .components(vec![
                menus::user_select_row(format!("r:pr:{raid_id}"), "Promote reserve → main", promote_opts),
                menus::user_select_row(format!("r:mr:{raid_id}"), "Promote main → reserve ", promote_to_reserve_opts),
                menus::user_select_row(format!("r:kk:{raid_id}"), "Kick user ", kick_opts),
                CreateActionRow::Buttons(vec![
                    CreateButton::new(format!("r:cx:{raid_id}"))
                        .label("Cancel Raid (DM all + delete in 1h)")
                        .style(ButtonStyle::Danger),
                    CreateButton::new(format!("r:cl:{raid_id}"))
                        .label("Close")
                        .style(ButtonStyle::Secondary),
                    CreateButton::new(format!("r:not:{raid_id}"))
                        .label("Notify All Participants ")
                        .style(ButtonStyle::Secondary),
                    CreateButton::new(format!("r:cho:{raid_id}"))
                        .label("Change Owner")
                        .style(ButtonStyle::Primary),
                ])
            ])
    )).await?;
    Ok(())
}

async fn owner_promote(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if raid.owner_id != it.user.id.get() as i64 { return Ok(()); }

    if let ComponentInteractionDataKind::StringSelect { values } = &it.data.kind {
        let Some(uid_s) = values.first() else { return Ok(()); };
        if uid_s == "none" { return Ok(()); }
        let uid: Uuid = uid_s.parse().ok().unwrap_or(Default::default());

        // is there room?
        let mains = repo::count_mains(&pool, raid_id).await? as i32;
        if mains >= raid.max_players {
            it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new().content("Main slots are full.")
            )).await?;
            return Ok(());
        }
        let target_user: Option<i64> = sqlx::query_scalar!(
            "SELECT user_id FROM raid_participants WHERE id = $1 AND raid_id = $2",
            uid, raid_id
        ).fetch_optional(&pool).await?;
        // promote the oldest reserve row for that user (prefer non-alt)
        let _ = sqlx::query!(
            r#"
            WITH c AS (
              SELECT id FROM raid_participants
              WHERE raid_id = $1 AND id = $2 AND is_main = FALSE
              ORDER BY is_alt ASC, joined_at ASC
              LIMIT 1
            )
            UPDATE raid_participants p
            SET is_main = TRUE, is_reserve = FALSE
            FROM c
            WHERE p.id = c.id
            "#,
            raid_id, uid
        ).execute(&pool).await?;

        if let Some(uid) = target_user {
            let when_local = raid.scheduled_for.with_timezone(&Warsaw).format("%Y-%m-%d %H:%M %Z");
            let msg = format!(
                "✅ You were **promoted to MAIN** for **{}** on {}.\nChannel: <#{}>",
                raid.raid_name, when_local, raid.channel_id as u64
            );
            dm_user(&ctx.http, uid as u64, msg).await;
        }
        // if it was an alt, ensure we don't exceed alt cap—(owner override leaves as-is)
        // refresh
        let parts = repo::list_participants(&pool, raid_id).await?;
        let embed = embeds::render_raid_embed(ctx, raid.guild_id as u64, &raid, &parts);
        ChannelId::new(raid.channel_id as u64)
            .edit_message(&ctx.http, raid.message_id as u64,
                          EditMessage::new().embed(embed).components(vec![menus::main_buttons_row(raid_id)])).await?;

        it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new().content("Promoted.")
        )).await?;
    }
    Ok(())
}
async fn owner_move_to_reserve(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if raid.owner_id != it.user.id.get() as i64 { return Ok(()); }
    if let ComponentInteractionDataKind::StringSelect { values } = &it.data.kind {
        let Some(uid_s) = values.first() else { return Ok(()); };
        if uid_s == "none" { return Ok(()); }
        let uid: Uuid = uid_s.parse().ok().unwrap_or(Default::default());

        // is there room?
        let target_user: Option<i64> = sqlx::query_scalar!(
            "SELECT user_id FROM raid_participants WHERE id = $1 AND raid_id = $2",
            uid, raid_id
        ).fetch_optional(&pool).await?;
        // promote the oldest reserve row for that user (prefer non-alt)
        let _ = sqlx::query!(
            r#"
            WITH c AS (
              SELECT id FROM raid_participants
              WHERE raid_id = $1 AND id = $2 AND is_main = TRUE
              ORDER BY is_alt ASC, joined_at ASC
              LIMIT 1
            )
            UPDATE raid_participants p
            SET is_main = FALSE, is_reserve = TRUE
            FROM c
            WHERE p.id = c.id
            "#,
            raid_id, uid
        ).execute(&pool).await?;
        if let Some(uid) = target_user {
            let when_local = raid.scheduled_for.with_timezone(&Warsaw).format("%Y-%m-%d %H:%M %Z");
            let msg = format!(
                "↩️ You were **moved to RESERVE** for **{}** on {}.\nChannel: <#{}>",
                raid.raid_name, when_local, raid.channel_id as u64
            );
            dm_user(&ctx.http, uid as u64, msg).await;
        }

        // if it was an alt, ensure we don't exceed alt cap—(owner override leaves as-is)
        // refresh
        let parts = repo::list_participants(&pool, raid_id).await?;
        let embed = embeds::render_raid_embed(ctx, raid.guild_id as u64, &raid, &parts);
        ChannelId::new(raid.channel_id as u64)
            .edit_message(&ctx.http, raid.message_id as u64,
                          EditMessage::new().embed(embed).components(vec![menus::main_buttons_row(raid_id)])).await?;

        it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new().content("Moved to reserve.")
        )).await?;
    }
    Ok(())
}

async fn owner_kick(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if raid.owner_id != it.user.id.get() as i64 { return Ok(()); }

    if let ComponentInteractionDataKind::StringSelect { values } = &it.data.kind {
        let Some(uid_s) = values.first() else { return Ok(()); };
        if uid_s == "none" { return Ok(()); }
        let uid: Uuid = uid_s.parse().ok().unwrap_or(Default::default());

        let _ = repo::remove_participant_by_id(&pool, raid_id, uid).await?;

        let should_try_promote = raid.priority_until.map(|t| chrono::Utc::now() >= t).unwrap_or(true);
        if should_try_promote {
            // zbuduj exclude_ids – użytkownicy z rolą RESERVE
            let mut exclude_ids: Vec<i64> = Vec::new();
            if let Some(gid) = it.guild_id {
                let roles_map = gid.roles(&ctx.http).await?;
                let reserve_role_name = std::env::var("RESERVE_ROLE_NAME").unwrap_or_else(|_| "reserve".to_string());
                let parts_for_check = repo::list_participants(&pool, raid_id).await?;
                for p in &parts_for_check {
                    if let Ok(member) = gid.member(&ctx.http, UserId::new(p.user_id as u64)).await {
                        let has_reserve = member.roles.iter().any(|rid| {
                            roles_map.get(rid).map_or(false, |r| r.name.eq_ignore_ascii_case(&reserve_role_name))
                        });
                        if has_reserve { exclude_ids.push(p.user_id); }
                    }
                }
            }
            repo::promote_reserves_global_order_excluding(
                &pool, raid_id, raid.max_players, raid.max_alts, &exclude_ids
            ).await?;
        }
        let parts = repo::list_participants(&pool, raid_id).await?;
        let embed = embeds::render_raid_embed(ctx, raid.guild_id as u64, &raid, &parts);
        ChannelId::new(raid.channel_id as u64)
            .edit_message(&ctx.http, raid.message_id as u64,
                          EditMessage::new().embed(embed).components(vec![menus::main_buttons_row(raid_id)])).await?;
        // refresh consolidated list if any
        let _ = crate::commands::raid::refresh_guild_raid_list_if_any(ctx, raid.guild_id as u64).await;
        it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new().content("Kicked.")
        )).await?;
    }
    Ok(())
}

async fn owner_cancel(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if raid.owner_id != it.user.id.get() as i64 { return Ok(()); }

    sqlx::query!("UPDATE raids SET is_active = FALSE WHERE id = $1", raid_id).execute(&pool).await?;

    let parts = repo::list_participants(&pool, raid_id).await?;
    for p in &parts {
        if let Ok(ch) = UserId::new(p.user_id as u64).create_dm_channel(&ctx.http).await {
            let _ = ch.id.send_message(&ctx.http, CreateMessage::new()
                .content(format!("Raid `{}` was cancelled.", raid.raid_name))
            ).await;
        }
    }

    let embed = CreateEmbed::new()
        .title(format!("Raid: {} (CANCELLED)", raid.raid_name))
        .description("This raid has been cancelled by the owner.");
    ChannelId::new(raid.channel_id as u64)
        .edit_message(&ctx.http, raid.message_id as u64, EditMessage::new().embed(embed)).await?;

    tokio::spawn({
        let http = ctx.http.clone();
        let channel_id = raid.channel_id as u64;
        async move {
            tokio::time::sleep(std::time::Duration::from_secs(7200)).await;
            let _ = ChannelId::new(channel_id).delete(&http).await;
        }
    });

    it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
        CreateInteractionResponseMessage::new().content("Raid cancelled. Channel will delete in ~2h.")
    )).await?;
    // refresh consolidated list if any
    let _ = crate::commands::raid::refresh_guild_raid_list_if_any(ctx, raid.guild_id as u64).await;
    Ok(())
}
async fn close_ephemeral(ctx: &Context, it: &ComponentInteraction) -> anyhow::Result<()> {
    // 1) Acknowledge quickly via UpdateMessage to avoid "Ta czynność się nie powiodła"
    //    Also remove components so it can't be clicked again.
    let ack = CreateInteractionResponse::UpdateMessage(
        CreateInteractionResponseMessage::new()
            .content("Closing…")
            .components(Vec::new()),
    );

    if let Err(err) = it.create_response(&ctx.http, ack).await {
        tracing::warn!("close_ephemeral: initial UpdateMessage failed: {err:?}");
        return Ok(());
    }

    // 2) Try to delete the original interaction response (works if this ephemeral is the original)
    match it.delete_response(&ctx.http).await {
        Ok(()) => return Ok(()),
        Err(err) => {
            tracing::debug!("close_ephemeral: delete_response failed: {err:?}; trying followup delete");

            // 3) If it was a followup ephemeral, delete that specific followup by message id
            if let Err(err2) = it.delete_followup(&ctx.http, it.message.id).await {
                tracing::warn!("close_ephemeral: delete_followup_message failed: {err2:?}; falling back to edit");

                // 4) Final fallback: keep a tiny stub with components removed
                let _ = it
                    .edit_response(
                        &ctx.http,
                        EditInteractionResponse::new()
                            .content("Closed.")
                            .components(Vec::new()),
                    )
                    .await;
            }
        }
    }

    Ok(())
}

fn human_time_left(when: DateTime<Utc>) -> String {
    let now = Utc::now();
    let mut d = when.signed_duration_since(now);
    if d.num_seconds() <= 0 {
        return "now".to_string();
    }

    let days = d.num_days();
    d -= DD::days(days);
    let hours = d.num_hours();
    d -= DD::hours(hours);
    let mins = d.num_minutes();

    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}d", days));
    }
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if mins > 0 && parts.len() < 2 {
        // Keep it concise: include minutes if useful and not too verbose.
        parts.push(format!("{}m", mins));
    }
    if parts.is_empty() {
        parts.push("<1m".to_string());
    }
    parts.join(" ")
}

async fn owner_change_start(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if raid.owner_id != it.user.id.get() as i64 {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Only the raid owner can change owner.").ephemeral(true)
        )).await?;
        return Ok(());
    }
    if !raid.is_active {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("This raid has been cancelled.").ephemeral(true)
        )).await?;
        return Ok(());
    }

    let gid = match it.guild_id {
        Some(g) => g,
        None => {
            it.create_response(&ctx.http, CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().content("Must be used in a guild.").ephemeral(true)
            )).await?;
            return Ok(());
        }
    };

    // Find organiser role id
    let roles_map = gid.roles(&ctx.http).await?;
    let organiser_role_id = match roles_map.iter()
        .find(|(_, r)| r.name.eq_ignore_ascii_case(ORGANISER_ROLE_NAME))
        .map(|(id, _)| *id)
    {
        Some(id) => id,
        None => {
            it.create_response(&ctx.http, CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(format!("Role `{}` not found on this server.", ORGANISER_ROLE_NAME))
                    .ephemeral(true)
            )).await?;
            return Ok(());
        }
    };

    // Collect members with organiser role (paginate; cap at 25 for Discord select)
    let mut after: Option<UserId> = None;
    let mut organisers: Vec<UserId> = Vec::new();
    loop {
        let chunk = gid.members(&ctx.http, Some(1000), after).await?;
        if chunk.is_empty() { break; }
        for m in &chunk {
            if m.user.id.get() as i64 != raid.owner_id && m.roles.contains(&organiser_role_id) {
                organisers.push(m.user.id);
            }
        }
        after = chunk.last().map(|m| m.user.id);
        if chunk.len() < 1000 { break; }
    }

    // Build options (label, value) — value = Discord user id as string
    // NOTE: Discord StringSelect allows max 25 options.
    organisers.sort_by_key(|u| u.get());
    let mut options: Vec<(String, String)> = Vec::new();
    for uid in organisers.iter().take(25) {
        let label = user_name_best(ctx, Some(gid.get()), uid.get() as i64).await;
        options.push((label, uid.get().to_string()));
    }

    if options.is_empty() {
        options.push((format!("No users with role {}", ORGANISER_ROLE_NAME), "none".into()));
    }

    OWNER_CHANGE.remove(&(it.user.id.get(), raid_id)); // reset previous pick if any

    it.create_response(&ctx.http, CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(format!("Pick a new owner (role: `{}`), then press **Transfer ownership**.", ORGANISER_ROLE_NAME))
            .ephemeral(true)
            .components(vec![
                menus::user_select_row(format!("r:chp:{raid_id}"), "New owner", options),
                CreateActionRow::Buttons(vec![
                    CreateButton::new(format!("r:chc:{raid_id}"))
                        .label("Transfer ownership")
                        .style(ButtonStyle::Primary),
                    CreateButton::new(format!("r:cl:{raid_id}"))
                        .label("Close")
                        .style(ButtonStyle::Secondary),
                ])
            ])
    )).await?;

    Ok(())
}

async fn owner_change_pick(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    // Store the selected Discord user id (as u64)
    if let ComponentInteractionDataKind::StringSelect { values } = &it.data.kind {
        let Some(sel) = values.first() else { return Ok(()); };
        if sel == "none" {
            it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new().content("No eligible users to pick.")
            )).await?;
            return Ok(());
        }
        if let Ok(uid64) = sel.parse::<u64>() {
            OWNER_CHANGE.insert((it.user.id.get(), raid_id), uid64);
            let picked = user_name_best(ctx, it.guild_id.map(|g| g.get()), uid64 as i64).await;
            it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new()
                    .content(format!("Selected new owner: **{}**. Now click **Transfer ownership**.", picked))
            )).await?;
        }
    }
    Ok(())
}

async fn owner_change_confirm(ctx: &Context, it: &ComponentInteraction, raid_id: Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let mut raid = repo::get_raid(&pool, raid_id).await?;
    if raid.owner_id != it.user.id.get() as i64 {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Only the raid owner can change owner.").ephemeral(true)
        )).await?;
        return Ok(());
    }
    if !raid.is_active {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("This raid has been cancelled.").ephemeral(true)
        )).await?;
        return Ok(());
    }

    let key = (it.user.id.get(), raid_id);
    let Some(&new_owner_u64) = OWNER_CHANGE.get(&key).as_deref() else {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("Pick a new owner first.")
                .ephemeral(true)
        )).await?;
        return Ok(());
    };

    if new_owner_u64 == it.user.id.get() {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("You’re already the owner.")
                .ephemeral(true)
        )).await?;
        return Ok(());
    }

    // Optional: verify the selected still has organiser role
    if let Some(gid) = it.guild_id {
        let roles_map = gid.roles(&ctx.http).await?;
        if let Some(role_id) = roles_map.iter()
            .find(|(_, r)| r.name.eq_ignore_ascii_case(ORGANISER_ROLE_NAME))
            .map(|(id, _)| *id)
        {
            if let Ok(member) = gid.member(&ctx.http, UserId::new(new_owner_u64)).await {
                if !member.roles.contains(&role_id) {
                    it.create_response(&ctx.http, CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content(format!("Selected user no longer has `{}` role.", ORGANISER_ROLE_NAME))
                            .ephemeral(true)
                    )).await?;
                    return Ok(());
                }
            }
        }
    }

    // Update DB
    sqlx::query!("UPDATE raids SET owner_id = $1 WHERE id = $2", new_owner_u64 as i64, raid_id)
        .execute(&pool)
        .await?;

    // Notify both owners
    let when_local = raid.scheduled_for.with_timezone(&Warsaw).format("%Y-%m-%d %H:%M %Z");
    let old_owner_u64 = raid.owner_id as u64;
    let new_owner_name = user_name_best(ctx, Some(raid.guild_id as u64), new_owner_u64 as i64).await;

    dm_user(&ctx.http, new_owner_u64, format!(
        "👑 You are now **owner** of raid **{}** ({}). Channel: <#{}>",
        raid.raid_name, when_local, raid.channel_id as u64
    )).await;

    dm_user(&ctx.http, old_owner_u64, format!(
        "↪️ Ownership of **{}** transferred to **{}**.",
        raid.raid_name, new_owner_name
    )).await;

    // Refresh message
    raid = repo::get_raid(&pool, raid_id).await?;
    let parts = repo::list_participants(&pool, raid_id).await?;
    let embed = embeds::render_raid_embed(ctx, raid.guild_id as u64, &raid, &parts);
    ChannelId::new(raid.channel_id as u64)
        .edit_message(&ctx.http, raid.message_id as u64,
                      EditMessage::new().embed(embed).components(vec![menus::main_buttons_row(raid_id)]))
        .await?;

    OWNER_CHANGE.remove(&key);

    it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
        CreateInteractionResponseMessage::new().content(format!("Ownership transferred to **{}**.", new_owner_name))
    )).await?;

    Ok(())
}
