use crate::db::repo;
use crate::handlers::pool_from_ctx;
use crate::ui::{embeds, menus};
use crate::utils::{from_user_id, parse_component_id};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use chrono::{DateTime,Duration as DD ,Utc};
use serenity::all::*;
use serenity::builder::{
    CreateInteractionResponse,
    CreateInteractionResponseMessage,
    EditInteractionResponse,
};
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
    if sel.class.is_none() || sel.sp.is_none() {
        it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new().content("Please choose both class and SP."),
        )).await?;
        sleep(Duration::from_secs(5)).await;
        let _ = it.delete_response(&ctx.http).await;
        return Ok(());
    }

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
                        if name=="c1-89" { has_c1_89 = true; }
                        allowed_by_crole = true;
                        break;
                    }
                }
            }
        }
    }

    if !allowed_by_crole {
        it.create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("You need role **c1-89** or **c90**, to join this raid.")
                    .ephemeral(true),
            ),
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


    // Priority: if window active and user lacks priority role → reserve
    let mut must_reserve = force_reserve;
    if let Some(until) = raid.priority_until {
        if chrono::Utc::now() < until {
            let has_priority = if let Some(gid) = it.guild_id {
                let member = gid.member(&ctx.http, it.user.id).await?;
                raid.priority_role_id
                    .map(|rid| member.roles.contains(&RoleId::new(rid as u64)))
                    .unwrap_or(false)
            } else { false };
            if !has_priority { must_reserve = true; }
        }
    }

    // Compute capacity
    let mains_cnt = repo::count_mains(&pool, raid_id).await? as i32;
    let free_main = (raid.max_players - mains_cnt).max(0);

    // Prepare fields
    let joined_as = format!("{} / {}", sel.class.unwrap(), sel.sp.unwrap());
    let is_alt_join = !sel.main;
    let requester_id = from_user_id(it.user.id);


    if is_alt_join && !repo::user_has_main(&pool, raid_id, requester_id).await? {
        it.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("You have to sign as a main before add alt.")
                .ephemeral(true)
        )).await?;
        return Ok(());
    }


    if is_alt_join {

        if !has_alt_allow_role {
            it.create_response(&ctx.http, CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(&format!("You are not allowed to sign in an alt. Missing role: {}", alt_allow_role_name))
                    .ephemeral(true)
            )).await?;
            return Ok(());
        }

        if !raid.allow_alts {
            it.create_response(&ctx.http, CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().content("Alts are disabled for this raid.").ephemeral(true)
            )).await?;
            return Ok(());
        }

        let user_alt_count = repo::alt_count_for_user(&pool, raid_id, from_user_id(it.user.id)).await? as i32;
        if user_alt_count >= raid.max_alts {
            it.create_response(&ctx.http, CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().content("You've reached your alt limit for this raid.").ephemeral(true)
            )).await?;
            return Ok(());
        }

        let alt_mains = repo::count_alt_mains(&pool, raid_id).await? as i32;
        let alt_slots_left = (raid.max_alts - alt_mains).max(0);

        let can_be_main = free_main > 0 && alt_slots_left > 0 && !must_reserve;
        let _ = repo::insert_alt(&pool, raid_id, from_user_id(it.user.id), joined_as, can_be_main,tag_suffix.clone()).await?;
    } else {
        // main join: ensure only one main per user
        let can_be_main = free_main > 0 && !must_reserve;
        let _ = repo::insert_or_replace_main(&pool, raid_id, from_user_id(it.user.id), joined_as, can_be_main,tag_suffix.clone()).await?;
    }

    // If window ended, try to promote reserves within alt cap
    let raid = repo::get_raid(&pool, raid_id).await?;
    if raid.priority_until.map(|t| chrono::Utc::now() >= t).unwrap_or(true) {
        let _ = repo::promote_reserves_with_alt_limits(&pool, raid_id, raid.max_players, raid.max_alts).await?;
    }

    // refresh message
    let raid = repo::get_raid(&pool, raid_id).await?;
    let parts = repo::list_participants(&pool, raid_id).await?;
    let embed = embeds::render_raid_embed(ctx, raid.guild_id as u64, &raid, &parts);

    ChannelId::new(raid.channel_id as u64)
        .edit_message(&ctx.http, raid.message_id as u64,
                      EditMessage::new()
                          .embed(embed)
                          .components(vec![menus::main_buttons_row(raid_id)])
        ).await?;

    it.create_response(&ctx.http, CreateInteractionResponse::UpdateMessage(
        CreateInteractionResponseMessage::new().content("You're signed in! ✅")
    )).await?;
    sleep(Duration::from_secs(5)).await;
    let _ = it.delete_response(&ctx.http).await;

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
    let removed_main = repo::remove_participant(&pool, raid_id, from_user_id(it.user.id)).await?;
    let user_id = from_user_id(it.user.id);
    let removed_alts = repo::remove_user_alts(&pool, raid_id, user_id).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    let time_left = human_time_left(raid.scheduled_for);
    let owner_id = UserId::new(raid.owner_id as u64);
    let dm = owner_id.create_dm_channel(&ctx.http).await?;
    if removed_main == 0 && removed_alts == 0 {
        // Already signed out — don't notify owner/channel again
        return refresh_message(ctx, it, raid_id, "You were already signed out.").await;
    }

    dm.id
        .send_message(
            &ctx.http,
            CreateMessage::new().content(format!(
                "Heads up: user <@{user}> signed off from raid \"{name}\".\nChannel: <#{chan}>\nStarts in: {left}",
                user = user_id,
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

    refresh_message(ctx, it, raid_id, "You have been signed out.").await
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


    let parts = repo::list_participants(&pool, raid_id).await?;
    // reserves-only options for Promote
    let mut promote_opts: Vec<(String,String)> = parts.iter()
        .filter(|p| !p.is_main)
        .map(|p| (format!("{} <@{}>{}", p.joined_as, p.user_id,
                          if p.is_alt {" (ALT)"} else {""}), p.id.to_string()))
        .collect();
    let mut promote_to_reserve_opts: Vec<(String,String)> = parts.iter()
        .filter(|p| p.is_main)
        .map(|p| (format!("{} <@{}>{}", p.joined_as, p.user_id,
                          if p.is_alt {" (ALT)"} else {""}), p.id.to_string()))
        .collect();
    if promote_to_reserve_opts.is_empty() {
        promote_to_reserve_opts.push(("No reserves".into(), "none".into()));
    }


    if promote_opts.is_empty() {
        promote_opts.push(("No reserves".into(), "none".into()));
    }

    // Any participants for Kick
    let mut kick_opts: Vec<(String,String)> = parts.iter()
        .map(|p| (format!("{} <@{}>{}{}", p.joined_as, p.user_id,
                          if p.is_main {" [MAIN]"} else {" [RES]"},
                          if p.is_alt {" (ALT)"} else {""}),
                  p.id.to_string()))
        .collect();

    if kick_opts.is_empty() {
        kick_opts.push(("No participants".into(), "none".into()));
    }

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

        let parts = repo::list_participants(&pool, raid_id).await?;
        let embed = embeds::render_raid_embed(ctx, raid.guild_id as u64, &raid, &parts);
        ChannelId::new(raid.channel_id as u64)
            .edit_message(&ctx.http, raid.message_id as u64,
                          EditMessage::new().embed(embed).components(vec![menus::main_buttons_row(raid_id)])).await?;
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
