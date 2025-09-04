use serenity::all::*;
use serenity::builder::{CreateChannel, CreateMessage};
use uuid::Uuid;

use crate::db::repo;
use crate::handlers::pool_from_ctx;
use crate::ui::{embeds, menus};
use crate::utils::{parse_raid_datetime, weekday_key,parse_list_unique};
use crate::tasks;
use crate::utils::extract_duration_hours;
use chrono_tz::Europe::Warsaw;

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

pub async fn handle(ctx: &Context, cmd: &CommandInteraction) -> anyhow::Result<()> {
    match cmd.data.name.as_str() {
        "raid" => handle_create(ctx, cmd).await,
        "raid_kick" => handle_kick(ctx, cmd).await,
        "raid_transfer" => handle_transfer(ctx, cmd).await,
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
            let csv = found_role_ids
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
    let duration_for_schedule:i64 = dur_h.ceil() as i64;
    tasks::schedule_auto_delete(
        ctx.http.clone(),
        raid_id,
        text_channel.id.get() as i64,
        scheduled_for + chrono::Duration::hours(duration_for_schedule) +chrono::Duration::minutes(20),
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
