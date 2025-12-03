use serenity::all::*;
use serenity::builder::{CreateChannel, CreateMessage, EditMessage};
use uuid::Uuid;

use crate::db::repo;
use crate::handlers::pool_from_ctx;
use crate::ui::{embeds, menus};
use crate::utils::{parse_raid_datetime, weekday_key,parse_list_unique, mention_user, ORGANISER_ROLE_NAME};
use crate::tasks;
use crate::utils::extract_duration_hours;
use chrono_tz::Europe::Warsaw;
use once_cell::sync::Lazy;
use dashmap::DashMap;

// In-memory registry of the guild's consolidated raids list message.
// Key: guild_id, Value: (channel_id, message_id)
pub static ALL_RAID_LIST_MSG: Lazy<DashMap<u64, (u64, u64)>> = Lazy::new(DashMap::new);

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

pub async fn handle(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    match cmd.data.name.as_str() {
        "raid" => handle_create(ctx, cmd).await,
        "raid_kick" => handle_kick(ctx, cmd).await,
        "raid_transfer" => handle_transfer(ctx, cmd).await,
        "role_add" => handle_role_add(ctx, cmd).await,
        "all_raid_list" => handle_all_raid_list(ctx, cmd).await,
        _ => Ok(())
    }
}

async fn handle_create(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
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
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Invalid date format. Use `HH:MM YYYY-MM-DD`.").ephemeral(true)
        )).await?;
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
                cmd.create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content(format!(
                                "None of the roles from `prioritylist` were found in this server: {}.",
                                listed
                            ))
                            .ephemeral(true),
                    ),
                )
                    .await?;
                return Ok(());
            }

            // Author must have at least one of the matched roles
            let member = gid.member(&ctx.http, cmd.user.id).await?;
            let has_any = member.roles.iter().any(|rid| found_role_ids.contains(rid));
            if !has_any {
                let listed = priority_role_name.join(", ");
                cmd.create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content(format!(
                                "You don't have any of the required roles for priority: {}.",
                                listed
                            ))
                            .ephemeral(true),
                    ),
                )
                    .await?;
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
            cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new().content("Use this in a server.").ephemeral(true)
            )).await?;
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

    cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content("Raid created!").ephemeral(true)
    )).await?;
    Ok(())
}

async fn handle_kick(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
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
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Invalid raid_id").ephemeral(true)
        )).await?; return Ok(());
    };

    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_uuid).await?;
    if raid.owner_id != cmd.user.id.get() as i64 {
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Only the raid owner can kick.").ephemeral(true)
        )).await?; return Ok(());
    }

    let Some(u) = user_id else {
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Missing user.").ephemeral(true)
        )).await?; return Ok(());
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

    cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content("Kicked.").ephemeral(true)
    )).await?;
    // refresh consolidated list if any
    let _ = refresh_guild_raid_list_if_any(ctx, raid.guild_id as u64).await;
    Ok(())
}

async fn handle_transfer(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
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
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Invalid raid_id").ephemeral(true)
        )).await?; return Ok(());
    };
    let pool = pool_from_ctx(ctx).await?;
    let raid = repo::get_raid(&pool, raid_id).await?;
    if raid.owner_id != cmd.user.id.get() as i64 {
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Only current owner can transfer.").ephemeral(true)
        )).await?; return Ok(());
    }
    let Some(new_owner) = new_owner else { return Ok(()); };
    sqlx::query!("UPDATE raids SET owner_id = $1 WHERE id = $2", new_owner.get() as i64, raid_id)
        .execute(&pool).await?;
    cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content("Ownership transferred.").ephemeral(true)
    )).await?;
    Ok(())
}
async fn handle_role_add(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
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
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Use this in a server.").ephemeral(true)
        )).await?; return Ok(());
    };

    let Some(user_id) = target_user else {
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Missing user.").ephemeral(true)
        )).await?; return Ok(());
    };

    // Permission: must have raid_organiser role
    let roles_map = gid.roles(&ctx.http).await?;
    let invoker = gid.member(&ctx.http, cmd.user.id).await?;
    let is_organiser = invoker.roles.iter().any(|rid| {
        roles_map.get(rid).map_or(false, |r| r.name.eq_ignore_ascii_case(ORGANISER_ROLE_NAME))
    });
    if !is_organiser {
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Only raid_organiser can use this.").ephemeral(true)
        )).await?; return Ok(());
    }

    // Resolve actual role name (reserve can be overridden by env)
    let wanted_name = if role_choice.eq_ignore_ascii_case("reserve") {
        std::env::var("RESERVE_ROLE_NAME").unwrap_or_else(|_| "reserve".to_string())
    } else {
        role_choice.clone()
    };

    let role_id = roles_map.iter().find_map(|(rid, r)| if r.name.eq_ignore_ascii_case(&wanted_name) { Some(*rid) } else { None });
    let Some(role_id) = role_id else {
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content(format!("Role '{}' not found on this server.", wanted_name)).ephemeral(true)
        )).await?; return Ok(());
    };

    let mut member = gid.member(&ctx.http, user_id).await?;
    let res = if action.eq_ignore_ascii_case("add") {
        member.add_role(&ctx.http, role_id).await
    } else if action.eq_ignore_ascii_case("remove") {
        member.remove_role(&ctx.http, role_id).await
    } else {
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new().content("Action must be 'add' or 'remove'.").ephemeral(true)
        )).await?; return Ok(());
    };

    match res {
        Ok(_) => {
            cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(format!(
                        "{} role '{}' for {}.",
                        if action.eq_ignore_ascii_case("add") { "Added" } else { "Removed" },
                        wanted_name,
                        mention_user(user_id.get() as i64)
                    ))
                    .ephemeral(true)
            )).await?;
        }
        Err(e) => {
            cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::new()
                    .content(format!("Failed to {} role: {}", action, e))
                    .ephemeral(true)
            )).await?;
        }
    }
    Ok(())
}

async fn handle_all_raid_list(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    let Some(gid) = cmd.guild_id else { return Ok(()); };
    let pool = pool_from_ctx(ctx).await?;
    let list_text = render_all_raids_list(ctx, &pool, gid.get()).await?;

    // Send a new message in the invoking channel and store mapping
    let msg = cmd.channel_id.send_message(&ctx.http, CreateMessage::new().content(list_text)).await?;
    ALL_RAID_LIST_MSG.insert(gid.get(), (cmd.channel_id.get(), msg.id.get()));

    // Best-effort ephemeral ack as well (optional)
    let _ = cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content("Posted/updated the raids list in this channel.").ephemeral(true)
    )).await;

    Ok(())
}

async fn render_all_raids_list(ctx: &Context, pool: &sqlx::PgPool, guild_id: u64) -> anyhow::Result<String> {
    let rows = repo::list_active_raids_by_guild(pool, guild_id as i64).await?;
    if rows.is_empty() {
        return Ok("Brak aktywnych rajdów.".to_string());
    }
    let mut out = String::from("Aktualne rajdy:\n");
    for r in rows {
        let filled = repo::count_mains(pool, r.id).await.unwrap_or(0);
        let chan_tag = format!("<#{}>", r.channel_id as u64);
        let owner = mention_user(r.created_by);
        let when_local = r.scheduled_for.with_timezone(&Warsaw).format("%Y-%m-%d %H:%M");
        out.push_str(&format!("• {} — {} — {}/{} — {} — {}\n", r.raid_name, owner, filled, r.max_players, when_local, chan_tag));
    }
    Ok(out)
}

pub async fn refresh_guild_raid_list_if_any(ctx: &Context, guild_id: u64) -> anyhow::Result<()> {
    if let Some(entry) = ALL_RAID_LIST_MSG.get(&guild_id) {
        let (chan_id, msg_id) = *entry.value();
        let pool = pool_from_ctx(ctx).await?;
        let content = render_all_raids_list(ctx, &pool, guild_id).await?;
        let _ = ChannelId::new(chan_id)
            .edit_message(&ctx.http, msg_id, EditMessage::new().content(content))
            .await;
    }
    Ok(())
}
