use serenity::all::{
    Command, CreateCommand, Context, CommandInteraction, CreateCommandOption, CommandOptionType,
    ChannelType, Permissions, CreateInteractionResponse, CreateInteractionResponseMessage,
    CommandDataOptionValue, GuildChannel, CreateMessage, ReactionType,
};
use chrono::{Utc, Weekday, Datelike, Duration};

/// Register `/raid` command with weekday and raid name options
pub async fn register(ctx: &Context) -> serenity::Result<()> {
    Command::create_global_command(
        &ctx.http,
        CreateCommand::new("raid")
            .description("Create a raid channel under a weekday category")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "weekday",
                    "Select the weekday category"
                )
                    .required(true)
                    .add_string_choice("Monday", "monday")
                    .add_string_choice("Tuesday", "tuesday")
                    .add_string_choice("Wednesday", "wednesday")
                    .add_string_choice("Thursday", "thursday")
                    .add_string_choice("Friday", "friday")
                    .add_string_choice("Saturday", "saturday")
                    .add_string_choice("Sunday", "sunday")
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "raid_name",
                    "Name of the raid"
                )
                    .required(true)
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "max_players",
                    "Max Main+Alt"
                )
                    .required(true)
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "allows_alt",
                    "Alt allowed?"
                ).required(true)
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "priority_list",
                    "Select priority Role"
                )
                .required(false)
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "priority_hour",
                    "select priority hour 1-24"
                )
                    .min_int_value(1)
                    .max_int_value(24)
            )
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "required_sp",
                    "Select priority Role"
                )
            )
    ).await?;
    Ok(())
}

/// Handle the `/raid` command - create channel and setup raid
pub async fn handle(
    ctx: &Context,
    cmd: &CommandInteraction,
) -> serenity::Result<()> {
    let guild_id = match cmd.guild_id {
        Some(g) => g,
        None => {
            let response = CreateInteractionResponseMessage::new()
                .content("This command must be used in a server.")
                .ephemeral(true);
            cmd.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await?;
            return Ok(());
        }
    };

    // Extract command options
    let selected_weekday = cmd.data.options.iter()
        .find(|opt| opt.name == "weekday")
        .and_then(|opt| match &opt.value {
            CommandDataOptionValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "monday".to_string());

    let raid_name = cmd.data.options.iter()
        .find(|opt| opt.name == "raid_name")
        .and_then(|opt| match &opt.value {
            CommandDataOptionValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "Unknown Raid".to_string());

    // Get all channels in the guild
    let channels = guild_id.channels(&ctx.http).await?;

    // Find the category channel matching the selected weekday
    let category_channel = channels.values()
        .find(|channel| {
            channel.kind == ChannelType::Category &&
                channel.name.to_lowercase().contains(&selected_weekday.to_lowercase())
        });

    let category_id = match category_channel {
        Some(category) => Some(category.id),
        None => {
            let response = CreateInteractionResponseMessage::new()
                .content(format!("❌ No category channel found with name: `{}`", selected_weekday))
                .ephemeral(true);
            cmd.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await?;
            return Ok(());
        }
    };

    // Calculate next date for the selected weekday
    let weekday_enum = string_to_weekday(&selected_weekday).unwrap();
    let next_date = get_next_weekday_date(weekday_enum);

    // Create channel name: "raid-name-YYYY-MM-DD"
    let channel_name = format!("{}-{}",
                               raid_name.to_lowercase().replace(" ", "-"),
                               next_date
    );

    // Check if channel already exists
    let existing_channel = channels.values()
        .find(|channel| {
            channel.name == channel_name &&
                channel.parent_id == category_id
        });

    if existing_channel.is_some() {
        let response = CreateInteractionResponseMessage::new()
            .content(format!("❌ Channel `{}` already exists under `{}`!", channel_name, selected_weekday))
            .ephemeral(true);
        cmd.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await?;
        return Ok(());
    }

    // Create the new channel under the category
    let builder = serenity::builder::CreateChannel::new(&channel_name)
        .kind(ChannelType::Text)
        .category(category_id.unwrap())
        .topic(format!("Raid: {} scheduled for {}", raid_name, next_date));

    let new_channel = guild_id.create_channel(&ctx.http, builder).await?;

    // Send initial message to the new channel
    let message = CreateMessage::new()
        .content(format!(
            "🎮 **{}** raid scheduled for **{}**!\n\n\
            📅 Date: {}\n\
            👤 Created by: <@{}>\n\n\
            React with ✅ to join this raid!",
            raid_name, selected_weekday, next_date, cmd.user.id
        ))
        .reactions(vec![ReactionType::Unicode("✅".to_string())]);

    new_channel.send_message(&ctx.http, message).await?;

    // Respond to the command
    let response = CreateInteractionResponseMessage::new()
        .content(format!(
            "✅ **Raid channel created!**\n\
            📁 Category: `{}`\n\
            📺 Channel: <#{}>\n\
            🎮 Raid: `{}`\n\
            📅 Date: `{}`",
            selected_weekday, new_channel.id, raid_name, next_date
        ));

    cmd.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await?;

    Ok(())
}

// Helper functions
fn get_next_weekday_date(target_weekday: Weekday) -> String {
    let today = Utc::now().date_naive();
    let today_weekday = today.weekday();

    let days_until_target = if target_weekday.number_from_monday() >= today_weekday.number_from_monday() {
        target_weekday.number_from_monday() - today_weekday.number_from_monday()
    } else {
        7 - today_weekday.number_from_monday() + target_weekday.number_from_monday()
    };

    let target_date = today + Duration::days(days_until_target as i64);
    target_date.format("%Y-%m-%d").to_string()
}

fn string_to_weekday(day: &str) -> Option<Weekday> {
    match day.to_lowercase().as_str() {
        "monday" => Some(Weekday::Mon),
        "tuesday" => Some(Weekday::Tue),
        "wednesday" => Some(Weekday::Wed),
        "thursday" => Some(Weekday::Thu),
        "friday" => Some(Weekday::Fri),
        "saturday" => Some(Weekday::Sat),
        "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}