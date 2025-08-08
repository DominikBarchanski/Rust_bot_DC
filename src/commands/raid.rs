use serenity::all::*;
use serenity::builder::{CreateChannel, CreateMessage};
use uuid::Uuid;

use crate::db::repo;
use crate::handlers::pool_from_ctx;
use crate::ui::{embeds, menus};
use crate::utils::{parse_raid_datetime, weekday_key};
use crate::tasks;

pub async fn register(ctx: &Context) -> anyhow::Result<()> {
    Command::create_global_command(
        &ctx.http,
        CreateCommand::new("raid")
            .description("Create a raid")
            // Required first
            .add_option(
                CreateCommandOption::new(CommandOptionType::String, "raid_name", "One of: arma_v2, pollu, arma")
                    .required(true)
                    .add_string_choice("arma_v2", "arma_v2")
                    .add_string_choice("pollu", "pollu")
                    .add_string_choice("arma", "arma")
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
    let mut priority_role_name: Option<String> = None;
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
            "prioritylist" => if let CommandDataOptionValue::String(s) = &opt.value { priority_role_name = Some(s.clone()); },
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

    let mut priority_role_id: Option<i64> = None;
    let mut priority_until: Option<chrono::DateTime<chrono::Utc>> = None;

    if priority {
        let role_name = priority_role_name.clone().unwrap_or_else(|| "Maraton".to_string());
        if let Some(gid) = cmd.guild_id {
            let roles_map = gid.roles(&ctx.http).await?;
            let role_id = roles_map
                .iter()
                .find_map(|(rid, r)| if r.name.eq_ignore_ascii_case(&role_name) { Some(*rid) } else { None });
            let Some(role_id) = role_id else {
                cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new().content(format!("Role `{}` not found.", role_name)).ephemeral(true)
                )).await?;
                return Ok(());
            };
            let member = gid.member(&ctx.http, cmd.user.id).await?;
            if !member.roles.contains(&role_id) {
                cmd.create_response(&ctx.http, CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::new().content(format!("You don't have the `{}` role required for priority.", role_name)).ephemeral(true)
                )).await?;
                return Ok(());
            }
            priority_role_id = Some(role_id.get() as i64);
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

    let chan_name = format!("{}-{}-{}", raid_name.replace('_', "-"),
                            scheduled_for.format("%Y%m%d"),
                            scheduled_for.format("%H%M"));

    let text_channel = match category_id {
        Some(cat) => {
            gid.create_channel(&ctx.http, CreateChannel::new(&chan_name).kind(ChannelType::Text).category(cat)).await?
        }
        None => {
            gid.create_channel(&ctx.http, CreateChannel::new(&chan_name).kind(ChannelType::Text)).await?
        }
    };

    let raid_id = Uuid::new_v4();
    let embed = embeds::render_new_raid_embed(&raid_name, &description, scheduled_for);
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
    tasks::schedule_auto_delete(
        ctx.http.clone(),
        raid_id,
        text_channel.id.get() as i64,
        scheduled_for + chrono::Duration::minutes(20),
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
