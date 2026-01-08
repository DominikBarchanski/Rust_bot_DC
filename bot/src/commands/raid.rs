use serenity::all::*;
use serenity::builder::{CreateChannel, CreateMessage, EditMessage, EditInteractionResponse};
use uuid::Uuid;

use crate::db::repo;
use crate::handlers::pool_from_ctx;
use crate::ui::{embeds, menus};
use crate::utils::{parse_raid_datetime, weekday_key,parse_list_unique, mention_user, ORGANISER_ROLE_NAME, PERMISSIONS_ROLE_NAME};
use crate::tasks;
use crate::utils::extract_duration_hours;
use chrono_tz::Europe::Warsaw;
use chrono::Datelike;
use once_cell::sync::Lazy;
use dashmap::DashMap;
use tokio::time::{sleep, Duration};

// Legacy in-memory registry (kept for compatibility but not authoritative).
// Authoritative registry lives in Redis + DB; we keep a tiny in-process debounce state below.
pub static ALL_RAID_LIST_MSG: Lazy<DashMap<u64, (u64, Vec<u64>)>> = Lazy::new(DashMap::new);

// Debounce flags per guild to coalesce frequent refreshes
static PENDING_REFRESH: Lazy<DashMap<u64, ()>> = Lazy::new(DashMap::new);

pub async fn register(ctx: &Context) -> anyhow::Result<()> {
    Command::create_global_command(
        &ctx.http,
        CreateCommand::new("raid")
            .description("Create a raid")
            // Required first
            //raid_name IN ('ArmaV2','Pollutus','Arma','Azgobas','Valehir','Alzanor','Hc_Azgobas','Hc_Valehir','Hc_Alzanor','Hc_A8-A6','Hc_A1-A5')
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "raid_name", "One of: arma_v2, pollu, arma")
                    .required(true)
                    .add_string_choice("ArmaV2", "ArmaV2")
                    .add_string_choice("Pollutus", "Pollutus")
                    .add_string_choice("Sky_Tower", "Sky_Tower")
                    .add_string_choice("Arma", "Arma")
                    .add_string_choice("Azgobas", "Azgobas")
                    .add_string_choice("Valehir", "Valehir")
                    .add_string_choice("Alzanor", "Alzanor")
                    .add_string_choice("Hc_Azgobas", "Hc_Azgobas")
                    .add_string_choice("Hc_Valehir", "Hc_Valehir")
                    .add_string_choice("Hc_Alzanor", "Hc_Alzanor")
                    .add_string_choice("Hc_A8-A6", "Hc_A8-A6")
                    .add_string_choice("Hc_A1-A5", "Hc_A1-A5")
                    .add_string_choice("Hc_A1-8", "Hc_A1-8")
                    .add_string_choice("Nezarun", "Nezarun")
                    .add_string_choice("Nezarun_v2", "Nezarun_v2")
            )
            .add_option(CreateCommandOption::new(CommandOptionType::String, "raid_date", "Format: HH:MM YYYY-MM-DD").required(true))
            .add_option(CreateCommandOption::new(CommandOptionType::Integer, "max_players", "Main slots").required(true))
            .add_option(CreateCommandOption::new(CommandOptionType::Boolean, "allow_alts", "Allow alts").required(true))
            .add_option(CreateCommandOption::new(CommandOptionType::Integer, "max_alts", "Alt slots").required(true))
            .add_option(CreateCommandOption::new(CommandOptionType::Boolean, "priority", "Enable priority role window").required(true))
            .add_option(CreateCommandOption::new(CommandOptionType::String, "description", "Short description").required(true))
            // Optional after
            .add_option(CreateCommandOption::new(CommandOptionType::String, "prioritylist", "Role name for priority (e.g., Maraton)"))
            .add_option(CreateCommandOption::new(CommandOptionType::Integer, "priority_hours", "How long priority lasts (hours)"))
    ).await?;
    Ok(())
}
fn emoji_and_slug(raid_choice: &str) -> (&'static str, String) {
    // Normalize name: lowercase and replace separators with hyphens
    let slug = raid_choice
        .to_lowercase()
        .replace(' ', "-")
        .replace('_', "-");

    // Pick emoji
    let emoji = match slug.as_str() {
        "armav2" => "🦾",
        "arma" => "🤖",
        "pollutus" => "🦠",
        "azgobas" => "🐉",
        "valehir" => "💀",
        "alzanor" => "🥶",
        "sky-tower" => "🗼",
        "nezarun" => "🔨",
        "nezarun-v2" => "🐙",
        s if s.starts_with("hc-") => "🔥", // Hc_* variants
        _ => "🏷️", // fallback
    };

    (emoji, slug)
}


pub async fn register_kick(ctx: &Context) -> anyhow::Result<()> {
    Command::create_global_command(
        &ctx.http,
        CreateCommand::new("raid_kick")
            .description("Kick a participant from a raid (owner only)")
            .add_option(CreateCommandOption::new(CommandOptionType::String, "raid_id", "Raid UUID").required(true))
            .add_option(CreateCommandOption::new(CommandOptionType::User, "user", "User to kick").required(true))
    ).await?;
    Ok(())
}

pub async fn register_transfer(ctx: &Context) -> anyhow::Result<()> {
    Command::create_global_command(
        &ctx.http,
        CreateCommand::new("raid_transfer")
            .description("Transfer raid ownership")
            .add_option(CreateCommandOption::new(CommandOptionType::String, "raid_id", "Raid UUID").required(true))
            .add_option(CreateCommandOption::new(CommandOptionType::User, "new_owner", "User").required(true))
    ).await?;
    Ok(())
}

pub async fn register_role_add(ctx: &Context) -> anyhow::Result<()> {
    Command::create_global_command(
        &ctx.http,
        CreateCommand::new("role_add")
            .description("Add or remove a predefined role to a user (raid_organiser only)")
            .add_option(CreateCommandOption::new(CommandOptionType::User, "user", "Target user").required(true))
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "action", "add or remove")
                    .required(true)
                    .add_string_choice("add", "add")
                    .add_string_choice("remove", "remove")
            )
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "role", "Role to set")
                    .required(true)
                    .add_string_choice("Maraton", "Maraton")
                    .add_string_choice("c90", "c90")
                    .add_string_choice("c1-89", "c1-89")
                    .add_string_choice("Alt_allow", "Alt_allow")
                    .add_string_choice("reserve", "reserve")
            )
    ).await?;
    Ok(())
}

pub async fn register_all_raid_list(ctx: &Context) -> anyhow::Result<()> {
    Command::create_global_command(
        &ctx.http,
        CreateCommand::new("all_raid_list")
            .description("Post a consolidated, auto-updating list of active raids in this channel")
    ).await?;
    Ok(())
}

// Register command to move the consolidated raids list to the current channel
pub async fn register_move_raid_list(ctx: &Context) -> anyhow::Result<()> {
    Command::create_global_command(
        &ctx.http,
        CreateCommand::new("move_raid_list_here")
            .description("Move the guild's consolidated raids list to this channel (raid_organiser only)")
    ).await?;
    Ok(())
}

pub async fn handle(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    match cmd.data.name.as_str() {
        "raid" => handle_create(ctx, cmd).await,
        "raid_kick" => handle_kick(ctx, cmd).await,
        "raid_transfer" => handle_transfer(ctx, cmd).await,
        "role_add" => handle_role_add(ctx, cmd).await,
        "all_raid_list" => handle_all_raid_list(ctx, cmd).await,
        "move_raid_list_here" => handle_move_raid_list(ctx, cmd).await,
        _ => Ok(())
    }
}

async fn handle_create(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    // Quick ephemeral ACK to avoid 10s latency errors
    let _ = cmd
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("⏳ Creating raid…")
                    .ephemeral(true),
            ),
        )
        .await;

    let mut raid_name = "arma_v2".to_string();
    let mut raid_date_str = String::new();
    let mut max_players: i64 = 12;
    let mut allow_alts = true;
    let mut max_alts: i64 = 1;
    let mut priority = false;
    let mut priority_role_name: Vec<String> = Vec::new();
    let mut priority_hours: Option<i64> = None;
    let mut description = String::new();

    for opt in &cmd.data.options {
        match opt.name.as_str() {
            "raid_name" => if let CommandDataOptionValue::String(s) = &opt.value { raid_name = s.clone(); },
            "raid_date" => if let CommandDataOptionValue::String(s) = &opt.value { raid_date_str = s.clone(); },
            "max_players" => if let CommandDataOptionValue::Integer(n) = &opt.value { max_players = *n; },
            "allow_alts" => if let CommandDataOptionValue::Boolean(b) = &opt.value { allow_alts = *b; },
            "max_alts" => if let CommandDataOptionValue::Integer(n) = &opt.value { max_alts = *n; },
            "priority" => if let CommandDataOptionValue::Boolean(b) = &opt.value { priority = *b; },
            "priority_hours" => if let CommandDataOptionValue::Integer(n) = &opt.value { priority_hours = Some(*n); },
            "prioritylist" => if let CommandDataOptionValue::String(s) = &opt.value { priority_role_name = parse_list_unique(s); },
            "description" => if let CommandDataOptionValue::String(s) = &opt.value { description = s.clone(); },
            _ => {}
        }
    }

    let Some(scheduled_for) = parse_raid_datetime(&raid_date_str) else {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new()
            .content("Invalid date format. Use `HH:MM YYYY-MM-DD`.")
        ).await?;
        return Ok(());
    };

    let mut priority_role_id: Option<Vec<i64>> = None;
    let mut priority_until: Option<chrono::DateTime<chrono::Utc>> = None;

    if priority {
        // default if user didn’t provide any list
        if priority_role_name.is_empty() {
            priority_role_name = vec!["Maraton".to_string()];
        }

        if let Some(gid) = cmd.guild_id {
            let roles_map = gid.roles(&ctx.http).await?;

            // Resolve provided names -> RoleId (case-insensitive)
            let mut found_role_ids: Vec<RoleId> = Vec::new();
            for name in &priority_role_name {
                if let Some((rid, _)) = roles_map
                    .iter()
                    .find(|(_, r)| r.name.eq_ignore_ascii_case(name))
                {
                    found_role_ids.push(*rid);
                }
            }

            if found_role_ids.is_empty() {
                let listed = priority_role_name.join(", ");
                cmd.edit_response(&ctx.http, EditInteractionResponse::new()
                    .content(format!(
                        "None of the roles from `prioritylist` were found in this server: {}.",
                        listed
                    ))
                ).await?;
                return Ok(());
            }

            // Author must have at least one of the matched roles
            let member = gid.member(&ctx.http, cmd.user.id).await?;
            let has_any = member.roles.iter().any(|rid| found_role_ids.contains(rid));
            if !has_any {
                let listed = priority_role_name.join(", ");
                cmd.edit_response(&ctx.http, EditInteractionResponse::new()
                    .content(format!(
                        "You don't have any of the required roles for priority: {}.",
                        listed
                    ))
                ).await?;
                return Ok(());
            }

            // === Store as CSV string ===
            let _csv = found_role_ids
                .iter()
                .map(|rid| rid.get().to_string())
                .collect::<Vec<_>>()
                .join(",");

            // Keep first role id for backward compat (if you still have the old column)
            let ids_vec_i64: Vec<i64> = found_role_ids.iter().map(|rid| rid.get() as i64).collect();
            priority_role_id = Some(ids_vec_i64);

            if let Some(h) = priority_hours {
                priority_until = Some(scheduled_for - chrono::Duration::hours(h));
            }
        }
    }


    let gid = match cmd.guild_id {
        Some(g) => g,
        None => {
            cmd.edit_response(&ctx.http, EditInteractionResponse::new()
                .content("Use this in a server.")
            ).await?;
            return Ok(());
        }
    };

    let weekday = weekday_key(scheduled_for);
    let guild = gid.to_partial_guild(&ctx.http).await?;
    let channels = guild.channels(&ctx.http).await?;
    let category_id = channels.values().find_map(|c| {
        if c.kind == ChannelType::Category && c.name.to_lowercase().contains(weekday) { Some(c.id) } else { None }
    });
    let when_local = scheduled_for.with_timezone(&Warsaw);
    let (emoji, name_slug) = emoji_and_slug(&raid_name);
    let date = when_local.format("%d-%m");
    let time = when_local.format("%H_%M");

    let chan_name = format!("{emoji}-{name_slug}-{date} at {time}",

    );


    let text_channel = match category_id {
        Some(cat) => {
            gid.create_channel(&ctx.http, CreateChannel::new(&chan_name).kind(ChannelType::Text).category(cat)).await?
        }
        None => {
            gid.create_channel(&ctx.http, CreateChannel::new(&chan_name).kind(ChannelType::Text)).await?
        }
    };

    let raid_id = Uuid::new_v4();
    let embed = embeds::render_new_raid_embed(&raid_name, &description, scheduled_for, &max_players);
    let (_desc_clean, dur_h) = extract_duration_hours(&description);
    let msg = text_channel.id.send_message(
        &ctx.http,
        CreateMessage::new()
            .embed(embed)
            .components(vec![menus::main_buttons_row(raid_id)])
    ).await?;

    repo::create_raid_with_id(
        &pool_from_ctx(ctx).await?,
        raid_id,
        gid.get() as i64,
        text_channel.id.get() as i64,
        msg.id.get() as i64,
        scheduled_for,
        cmd.user.id.get() as i64,
        cmd.user.id.get() as i64,
        description,
        vec![],
        priority,
        raid_name,
        max_players as i32,
        allow_alts,
        max_alts as i32,
        priority_role_id,
        priority_until,
    ).await?;

    if let Some(until) = priority_until {
        tasks::schedule_priority_promotion(
            ctx.http.clone(),
            pool_from_ctx(ctx).await?,
            raid_id,
            gid.get() as i64,
            text_channel.id.get() as i64,
            msg.id.get() as i64,
            until,
        );
    }

    // Try refresh consolidated list (if exists in this guild)
    let _ = refresh_guild_raid_list_if_any(ctx, gid.get()).await;
    let duration_for_schedule:i64 = dur_h.ceil() as i64;
    tasks::schedule_auto_delete(
        ctx.http.clone(),
        pool_from_ctx(ctx).await?,   // <—
        raid_id,
        text_channel.id.get() as i64,
        scheduled_for + chrono::Duration::hours(duration_for_schedule) + chrono::Duration::minutes(20),
    );
    tasks::schedule_raid_15m_reminder(
        ctx.http.clone(),
        pool_from_ctx(ctx).await?,
        raid_id,
        scheduled_for - chrono::Duration::minutes(15),
    );

    cmd.edit_response(&ctx.http, EditInteractionResponse::new()
        .content("Raid created!")
    ).await?;
    Ok(())
}

async fn handle_kick(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    // Quick ACK to prevent 10s timeout
    let _ = cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content("⏳ Processing…").ephemeral(true)
    )).await;

    let mut raid_id_s = String::new();
    let mut user_id: Option<UserId> = None;
    for o in &cmd.data.options {
        match o.name.as_str() {
            "raid_id" => if let CommandDataOptionValue::String(s) = &o.value { raid_id_s = s.clone(); },
            "user" => if let CommandDataOptionValue::User(u) = &o.value { user_id = Some(*u); },
            _ => {}
        }
    }

    let Ok(raid_uuid) = Uuid::parse_str(&raid_id_s) else {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Invalid raid_id")).await?; return Ok(());
    };

    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_uuid).await?;
    if raid.owner_id != cmd.user.id.get() as i64 {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Only the raid owner can kick.")).await?; return Ok(());
    }

    let Some(u) = user_id else {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Missing user.")).await?; return Ok(());
    };

    let _ = repo::remove_participant(&pool, raid_uuid, u.get() as i64).await?;

    let parts = repo::list_participants(&pool, raid_uuid).await?;
    let embed = crate::ui::embeds::render_raid_embed(ctx, raid.guild_id as u64, &raid, &parts);
    ChannelId::new(raid.channel_id as u64)
        .edit_message(&ctx.http, raid.message_id as u64,
                      serenity::builder::EditMessage::new()
                          .embed(embed)
                          .components(vec![crate::ui::menus::main_buttons_row(raid_uuid)]))
        .await?;

    cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Kicked.")).await?;
    // refresh consolidated list if any
    let _ = refresh_guild_raid_list_if_any(ctx, raid.guild_id as u64).await;
    Ok(())
}

async fn handle_transfer(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    // Quick ACK
    let _ = cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content("⏳ Processing…").ephemeral(true)
    )).await;
    let mut raid_s = String::new();
    let mut new_owner: Option<UserId> = None;
    for o in &cmd.data.options {
        match o.name.as_str() {
            "raid_id" => if let CommandDataOptionValue::String(s) = &o.value { raid_s = s.clone(); },
            "new_owner" => if let CommandDataOptionValue::User(u) = &o.value { new_owner = Some(*u); },
            _ => {}
        }
    }
    let Ok(raid_id) = Uuid::parse_str(&raid_s) else {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Invalid raid_id")).await?; return Ok(());
    };
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if raid.owner_id != cmd.user.id.get() as i64 {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Only current owner can transfer.")).await?; return Ok(());
    }
    let Some(new_owner) = new_owner else { return Ok(()); };
    sqlx::query!("UPDATE raids SET owner_id = $1 WHERE id = $2", new_owner.get() as i64, raid_id)
        .execute(&pool).await?;
    cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Ownership transferred.")).await?;
    Ok(())
}
async fn handle_role_add(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    // Quick ACK
    let _ = cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content("⏳ Processing…").ephemeral(true)
    )).await;
    // Parse options
    let mut target_user: Option<UserId> = None;
    let mut action: String = String::new();
    let mut role_choice: String = String::new();
    for o in &cmd.data.options {
        match o.name.as_str() {
            "user" => if let CommandDataOptionValue::User(u) = &o.value { target_user = Some(*u); },
            "action" => if let CommandDataOptionValue::String(s) = &o.value { action = s.clone(); },
            "role" => if let CommandDataOptionValue::String(s) = &o.value { role_choice = s.clone(); },
            _ => {}
        }
    }

    let Some(gid) = cmd.guild_id else {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Use this in a server.")).await?; return Ok(());
    };

    let Some(user_id) = target_user else {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Missing user.")).await?; return Ok(());
    };

    // Permission: must have raid_organiser role
    let roles_map = gid.roles(&ctx.http).await?;
    let invoker = gid.member(&ctx.http, cmd.user.id).await?;
    let is_organiser = invoker.roles.iter().any(|rid| {
        roles_map.get(rid).map_or(false, |r| r.name.eq_ignore_ascii_case(ORGANISER_ROLE_NAME))
    });
    if !is_organiser {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Only raid_organiser can use this.")).await?; return Ok(());
    }

    // Resolve actual role name (reserve can be overridden by env)
    let wanted_name = if role_choice.eq_ignore_ascii_case("reserve") {
        std::env::var("RESERVE_ROLE_NAME").unwrap_or_else(|_| "reserve".to_string())
    } else {
        role_choice.clone()
    };

    let role_id = roles_map.iter().find_map(|(rid, r)| if r.name.eq_ignore_ascii_case(&wanted_name) { Some(*rid) } else { None });
    let Some(role_id) = role_id else {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content(format!("Role '{}' not found on this server.", wanted_name))).await?; return Ok(());
    };

    let member = gid.member(&ctx.http, user_id).await?;
    let res = if action.eq_ignore_ascii_case("add") {
        member.add_role(&ctx.http, role_id).await
    } else if action.eq_ignore_ascii_case("remove") {
        member.remove_role(&ctx.http, role_id).await
    } else {
        cmd.edit_response(&ctx.http, EditInteractionResponse::new().content("Action must be 'add' or 'remove'.")).await?; return Ok(());
    };

    match res {
        Ok(_) => {
            cmd.edit_response(&ctx.http, EditInteractionResponse::new()
                .content(format!(
                    "{} role '{}' for {}.",
                    if action.eq_ignore_ascii_case("add") { "Added" } else { "Removed" },
                    wanted_name,
                    mention_user(user_id.get() as i64)
                ))
            ).await?;
        }
        Err(e) => {
            cmd.edit_response(&ctx.http, EditInteractionResponse::new()
                .content(format!("Failed to {} role: {}", action, e))
            ).await?;
        }
    }
    Ok(())
}

async fn handle_all_raid_list(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    let Some(gid) = cmd.guild_id else { return Ok(()); };

    // Quick ephemeral ACK to avoid 10s latency errors
    let _ = cmd
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("⏳ Generating the raids list…")
                    .ephemeral(true),
            ),
        )
        .await;

    let pool = crate::handlers::pool_from_ctx(ctx).await?;
    let redis = crate::handlers::redis_from_ctx(ctx).await?;

    // Check existing mapping from Redis, fallback to DB
    let existing = match crate::redis_ext::get_guild_list(&redis, gid.get()).await? {
        Some(m) => Some(m),
        None => match crate::db::repo::get_guild_raid_list(&pool, gid.get() as i64).await? {
            Some(r) => {
                let ids_u64: Vec<u64> = r.message_ids.iter().map(|i| *i as u64).collect();
                Some((r.channel_id as u64, ids_u64))
            }
            None => None,
        },
    };

    if let Some((other_chan, _ids)) = existing {
        if other_chan != cmd.channel_id.get() {
            let _ = cmd
                .edit_response(
                    &ctx.http,
                    EditInteractionResponse::new().content(format!(
                        "❌ Lista dla tego serwera już istnieje w kanale <#{}>. Aby przenieść, użyj komendy: /move_raid_list_here w docelowym kanale.",
                        other_chan
                    )),
                )
                .await;
            return Ok(());
        }

        // Same channel -> just trigger refresh/update
        trigger_refresh(ctx, gid.get()).await;
        let _ = cmd
            .edit_response(&ctx.http, EditInteractionResponse::new().content("Zaktualizowano listę rajdów w tym kanale."))
            .await;
        return Ok(());
    }

    // No mapping yet: render and post
    let chunks = render_all_raids_list(ctx, &pool, gid.get()).await?;
    let mut ids: Vec<u64> = Vec::new();
    for c in &chunks {
        let m = cmd
            .channel_id
            .send_message(&ctx.http, CreateMessage::new().content(c.clone()))
            .await?;
        ids.push(m.id.get());
    }

    // Persist to Redis + DB and local cache
    crate::redis_ext::set_guild_list(&redis, gid.get(), cmd.channel_id.get(), &ids).await?;
    let ids_i64: Vec<i64> = ids.iter().map(|i| *i as i64).collect();
    crate::db::repo::upsert_guild_raid_list(&pool, gid.get() as i64, cmd.channel_id.get() as i64, &ids_i64).await?;
    ALL_RAID_LIST_MSG.insert(gid.get(), (cmd.channel_id.get(), ids));

    // Finalize the ephemeral response
    let _ = cmd
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new()
                .content("Posted/updated the raids list in this channel."),
        )
        .await;

    Ok(())
}

async fn handle_move_raid_list(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    let Some(gid) = cmd.guild_id else { return Ok(()); };

    // Quick ephemeral ACK
    let _ = cmd
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content("⏳ Moving the raids list here…")
                    .ephemeral(true),
            ),
        )
        .await;

    // Permission: must have raid_organiser role
    let roles_map = gid.roles(&ctx.http).await?;
    let invoker = gid.member(&ctx.http, cmd.user.id).await?;
    let is_organiser = invoker.roles.iter().any(|rid| {
        roles_map
            .get(rid)
            .map_or(false, |r| r.name.eq_ignore_ascii_case(PERMISSIONS_ROLE_NAME))
    });
    if !is_organiser {
        cmd
            .edit_response(
                &ctx.http,
                EditInteractionResponse::new()
                    .content("Only raid_organiser can use this."),
            )
            .await?;
        return Ok(());
    }

    let pool = crate::handlers::pool_from_ctx(ctx).await?;
    let redis = crate::handlers::redis_from_ctx(ctx).await?;

    // Render current list
    let chunks = render_all_raids_list(ctx, &pool, gid.get()).await?;

    // Find existing mapping
    let existing = match crate::redis_ext::get_guild_list(&redis, gid.get()).await? {
        Some(m) => Some(m),
        None => match crate::db::repo::get_guild_raid_list(&pool, gid.get() as i64).await? {
            Some(r) => {
                let ids_u64: Vec<u64> = r.message_ids.iter().map(|i| *i as u64).collect();
                Some((r.channel_id as u64, ids_u64))
            }
            None => None,
        },
    };

    // Post in the target channel (current interaction channel)
    let mut new_ids: Vec<u64> = Vec::new();
    for c in &chunks {
        let m = cmd
            .channel_id
            .send_message(&ctx.http, CreateMessage::new().content(c.clone()))
            .await?;
        new_ids.push(m.id.get());
    }

    // Persist new mapping
    crate::redis_ext::set_guild_list(&redis, gid.get(), cmd.channel_id.get(), &new_ids).await?;
    let ids_i64: Vec<i64> = new_ids.iter().map(|i| *i as i64).collect();
    crate::db::repo::upsert_guild_raid_list(&pool, gid.get() as i64, cmd.channel_id.get() as i64, &ids_i64).await?;

    // Try removing the old messages if mapping existed elsewhere
    if let Some((old_chan, old_ids)) = existing {
        if old_chan != cmd.channel_id.get() {
            let old_channel = ChannelId::new(old_chan);
            for mid in old_ids {
                let _ = old_channel.delete_message(&ctx.http, mid).await;
            }
        }
    }

    // Finalize
    cmd
        .edit_response(
            &ctx.http,
            EditInteractionResponse::new().content("Przeniesiono listę rajdów do tego kanału."),
        )
        .await?;

    Ok(())
}

fn day_labels() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("Mon", "Poniedziałek", "Monday"),
        ("Tue", "Wtorek", "Tuesday"),
        ("Wed", "Środa", "Wednesday"),
        ("Thu", "Czwartek", "Thursday"),
        ("Fri", "Piątek", "Friday"),
        ("Sat", "Sobota", "Saturday"),
        ("Sun", "Niedziela", "Sunday"),
    ]
}

async fn render_all_raids_list(_ctx: &Context, pool: &sqlx::PgPool, guild_id: u64) -> anyhow::Result<Vec<String>> {
    let rows = repo::list_active_raids_by_guild(pool, guild_id as i64).await?;
    if rows.is_empty() {
        return Ok(vec!["Brak aktywnych rajdów.".to_string()]);
    }

    // Group raids by weekday in Warsaw timezone
    let mut by_day: [Vec<String>; 7] = Default::default();
    for r in rows {
        let when_local = r.scheduled_for.with_timezone(&Warsaw);
        let weekday = when_local.weekday();
        let idx = match weekday {
            chrono::Weekday::Mon => 0,
            chrono::Weekday::Tue => 1,
            chrono::Weekday::Wed => 2,
            chrono::Weekday::Thu => 3,
            chrono::Weekday::Fri => 4,
            chrono::Weekday::Sat => 5,
            chrono::Weekday::Sun => 6,
        };
        let filled = repo::count_mains(pool, r.id).await.unwrap_or(0);
        let chan_tag = format!("<#{}>", r.channel_id as u64);
        // No owner, no time — just name, count and channel
        by_day[idx].push(format!("• {} — {}/{} — {}", r.raid_name, filled, r.max_players, chan_tag));
    }

    // Build full template with bilingual headers and footers
    let mut sections: Vec<String> = Vec::new();
    let header = "# :flag_pl: Rajdy na następny tydzień zostały rozpisane.\n# :flag_gb: Raids for next week have been organised.\n\n";
    sections.push(header.to_string());

    let labels = day_labels();
    for (i, (_abbr, pl, en)) in labels.iter().enumerate() {
        let mut s = String::new();
        s.push_str(&format!("**{} - {} **\n", pl, en));
        if by_day[i].is_empty() {
            // leave empty (no #bramki placeholder)
        } else {
            s.push_str(&by_day[i].join("\n"));
            s.push('\n');
        }
        s.push('\n');
        sections.push(s);
    }

    let footer = "**Niektóre rajdy mogą zostać dopisane w późniejszym terminie.**\n**Some raids may be added at a later date.**\n";
    sections.push(footer.to_string());

    // Split into up to two messages under 2000 chars
    const LIM: usize = 1900; // safety margin below 2000
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for sec in sections {
        if cur.len() + sec.len() <= LIM {
            cur.push_str(&sec);
        } else {
            if cur.is_empty() {
                // section alone too big: hard-truncate
                let mut truncated = sec.chars().take(LIM - 1).collect::<String>();
                truncated.push('\n');
                chunks.push(truncated);
            } else {
                chunks.push(cur);
                cur = String::new();
                cur.push_str(&sec);
            }
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }

    if chunks.len() > 2 {
        // Keep only two; merge the tail into the second with truncation
        let mut second = String::new();
        for ch in chunks.iter().skip(1) {
            if second.len() + ch.len() <= LIM {
                second.push_str(ch);
            } else {
                let space = LIM.saturating_sub(second.len());
                if space > 4 {
                    let mut part = ch.chars().take(space - 4).collect::<String>();
                    part.push_str("\n…");
                    second.push_str(&part);
                }
                break;
            }
        }
        return Ok(vec![chunks[0].clone(), second]);
    }

    Ok(chunks)
}

pub async fn refresh_guild_raid_list_if_any(ctx: &Context, guild_id: u64) -> anyhow::Result<()> {
    // Backward-compatible shim: redirect to debounced refresh
    trigger_refresh(ctx, guild_id).await;
    Ok(())
}

pub async fn refresh_all_guild_lists_if_any(ctx: &Context) -> anyhow::Result<()> {
    let pool = crate::handlers::pool_from_ctx(ctx).await?;
    let rows = crate::db::repo::list_all_guild_raid_lists(&pool).await?;
    for r in rows {
        // spawn per guild to avoid blocking
        let ctxc = ctx.clone();
        tokio::spawn(async move {
            let _ = do_refresh(&ctxc, r.guild_id as u64).await;
        });
    }
    Ok(())
}

pub async fn trigger_refresh(ctx: &Context, guild_id: u64) {
    let first = PENDING_REFRESH.insert(guild_id, ()).is_none();
    if first {
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(2)).await; // debounce bursty events
            let _ = do_refresh(&ctx2, guild_id).await;
            PENDING_REFRESH.remove(&guild_id);
        });
    }
}

// Immediate, non-debounced refresh. Use after a successful write that should reflect instantly.
pub async fn force_refresh_guild_raid_list(ctx: &Context, guild_id: u64) -> anyhow::Result<()> {
    do_refresh(ctx, guild_id).await
}

async fn do_refresh(ctx: &Context, guild_id: u64) -> anyhow::Result<()> {
    use crate::handlers::{pool_from_ctx, redis_from_ctx};
    let pool = pool_from_ctx(ctx).await?;
    let redis = redis_from_ctx(ctx).await?;

    // Resolve mapping from Redis, fallback DB
    let mapping = match crate::redis_ext::get_guild_list(&redis, guild_id).await? {
        Some(m) => Some(m),
        None => match crate::db::repo::get_guild_raid_list(&pool, guild_id as i64).await? {
            Some(r) => Some((r.channel_id as u64, r.message_ids.iter().map(|i| *i as u64).collect())),
            None => None,
        },
    };
    let Some((chan_id, mut msg_ids)) = mapping else { return Ok(()); };

    let chunks = render_all_raids_list(ctx, &pool, guild_id).await?;
    let channel = ChannelId::new(chan_id);

    // Try to edit appropriately; on failures recreate
    let result: Result<(), serenity::Error> = match (msg_ids.len(), chunks.len()) {
        (1, 1) => {
            channel
                .edit_message(&ctx.http, msg_ids[0], EditMessage::new().content(chunks[0].clone()))
                .await
                .map(|_| ())
        }
        (1, 2) => {
            let _ = channel
                .edit_message(&ctx.http, msg_ids[0], EditMessage::new().content(chunks[0].clone()))
                .await;
            match channel
                .send_message(&ctx.http, CreateMessage::new().content(chunks[1].clone()))
                .await
            {
                Ok(m) => { msg_ids.push(m.id.get()); Ok::<(), serenity::Error>(()) }
                Err(e) => Err(e),
            }
        }
        (2, 1) => {
            let _ = channel
                .edit_message(&ctx.http, msg_ids[0], EditMessage::new().content(chunks[0].clone()))
                .await;
            let _ = channel.delete_message(&ctx.http, msg_ids[1]).await;
            msg_ids.truncate(1);
            Ok(())
        }
        (2, 2) => {
            let _ = channel
                .edit_message(&ctx.http, msg_ids[0], EditMessage::new().content(chunks[0].clone()))
                .await;
            channel
                .edit_message(&ctx.http, msg_ids[1], EditMessage::new().content(chunks[1].clone()))
                .await
                .map(|_| ())
        }
        _ => {
            // Recreate fresh
            for mid in msg_ids.iter() { let _ = channel.delete_message(&ctx.http, *mid).await; }
            msg_ids.clear();
            for c in chunks {
                let m = channel
                    .send_message(&ctx.http, CreateMessage::new().content(c))
                    .await?;
                msg_ids.push(m.id.get());
            }
            Ok(())
        }
    };

    if result.is_err() {
        // Fallback: recreate everything
        for mid in msg_ids.iter() { let _ = channel.delete_message(&ctx.http, *mid).await; }
        msg_ids.clear();
        let chunks2 = render_all_raids_list(ctx, &pool, guild_id).await?;
        for c in chunks2 {
            let m = channel
                .send_message(&ctx.http, CreateMessage::new().content(c))
                .await?;
            msg_ids.push(m.id.get());
        }
    }

    // Persist state (DB + Redis)
    let ids_i64: Vec<i64> = msg_ids.iter().map(|i| *i as i64).collect();
    let _ = crate::db::repo::upsert_guild_raid_list(&pool, guild_id as i64, chan_id as i64, &ids_i64).await;
    let _ = crate::redis_ext::set_guild_list(&redis, guild_id, chan_id, &msg_ids).await;
    // update compatibility cache
    ALL_RAID_LIST_MSG.insert(guild_id, (chan_id, msg_ids));
    Ok(())
}
