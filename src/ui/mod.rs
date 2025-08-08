use serenity::async_trait;
use serenity::model::prelude::*;
use serenity::builder::CreateCommand;
use serenity::model::application::CommandInteraction;
use serenity::prelude::*;
use chrono::{DateTime, Datelike, Duration, Utc, Weekday};
use serenity::all::{CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage};
use sqlx::PgPool;

pub struct Handler {
    pub pool: sqlx::PgPool,
}

impl Handler {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }

    pub async fn create_raid_in_db(
        &self,
        guild_id: i64,
        channel_id: i64,
        message_id: i64,
        scheduled_for: DateTime<Utc>,
        created_by: i64,
        owner_id: i64,
        description: String,
        is_priority: bool,
        priority_list: String,
    ) -> anyhow::Result<crate::db::models::Raid> {
        crate::db::repo::create_raid(
            &self.pool,
            guild_id,
            channel_id,
            scheduled_for,
            created_by,
            description,
            owner_id,
            message_id,
            is_priority,
            priority_list,
        ).await
    }

}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
        // Register commands
        if let Err(e) = crate::commands::register_commands(&ctx).await {
            eprintln!("Failed to register commands: {}", e);
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        if let Interaction::Command(cmd) = interaction {
            match cmd.data.name.as_str() {
                "ping" => {
                    let _ = crate::commands::ping::run(&ctx, &cmd).await;
                }
                "raid" => {
                    let _ = self.handle_with_db(&ctx, &cmd, &self.pool).await;
                }

                // future commands
                _ => {}
            }
        }
    }
}
impl Handler {
    async fn handle_with_db(
        &self,
        ctx: &Context,
        cmd: &CommandInteraction,
        pool: &PgPool,
    ) -> anyhow::Result<()> {
        println!("🎮 Raid command received from user: {}", cmd.user.name);

        let guild_id = match cmd.guild_id {
            Some(g) => {
                println!("📍 Guild ID: {}", g.get());
                g
            },
            None => {
                println!("❌ Command used outside of guild");
                let response = CreateInteractionResponseMessage::new()
                    .content("This command must be used in a server.")
                    .ephemeral(true);
                cmd.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await?;
                return Ok(());
            }
        };

        println!("📋 Extracting command options...");

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

        println!("📝 Options extracted - Weekday: {}, Raid: {}", selected_weekday, raid_name);

        // Get all channels in the guild
        println!("🔍 Fetching guild channels...");
        let channels = match guild_id.channels(&ctx.http).await {
            Ok(channels) => {
                println!("✅ Found {} channels", channels.len());
                channels
            },
            Err(e) => {
                eprintln!("❌ Failed to fetch channels: {}", e);
                return Err(e.into());
            }
        };

        // Find the category channel matching the selected weekday
        println!("🔍 Looking for category: {}", selected_weekday);
        let category_channel = channels.values()
            .find(|channel| {
                let matches = channel.kind == ChannelType::Category &&
                    channel.name.to_lowercase().contains(&selected_weekday.to_lowercase());
                if matches {
                    println!("✅ Found matching category: {}", channel.name);
                }
                matches
            });

        let category_id = match category_channel {
            Some(category) => {
                println!("✅ Using category ID: {}", category.id);
                Some(category.id)
            },
            None => {
                println!("❌ No category found for: {}", selected_weekday);
                let response = CreateInteractionResponseMessage::new()
                    .content(format!("❌ No category channel found with name: `{}`", selected_weekday))
                    .ephemeral(true);
                cmd.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await?;
                return Ok(());
            }
        };

        // Calculate next date for the selected weekday
        println!("📅 Calculating next weekday date...");
        let weekday_enum = string_to_weekday(&selected_weekday).unwrap();
        let next_date = get_next_weekday_date(weekday_enum);
        println!("📅 Next date: {}", next_date);

        // Create channel name: "raid-name-YYYY-MM-DD"
        let channel_name = format!("{}-{}",
                                   raid_name.to_lowercase().replace(" ", "-"),
                                   next_date
        );
        println!("📺 Channel name will be: {}", channel_name);

        // Check if channel already exists
        let existing_channel = channels.values()
            .find(|channel| {
                channel.name == channel_name &&
                    channel.parent_id == category_id
            });

        if existing_channel.is_some() {
            println!("❌ Channel already exists: {}", channel_name);
            let response = CreateInteractionResponseMessage::new()
                .content(format!("❌ Channel `{}` already exists under `{}`!", channel_name, selected_weekday))
                .ephemeral(true);
            cmd.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await?;
            return Ok(());
        }

        // Create the new channel under the category
        println!("🏗️ Creating new channel...");
        let builder = serenity::builder::CreateChannel::new(&channel_name)
            .kind(ChannelType::Text)
            .category(category_id.unwrap())
            .topic(format!("Raid: {} scheduled for {}", raid_name, next_date));

        let new_channel = match guild_id.create_channel(&ctx.http, builder).await {
            Ok(channel) => {
                println!("✅ Channel created: {} (ID: {})", channel.name, channel.id);
                channel
            },
            Err(e) => {
                eprintln!("❌ Failed to create channel: {}", e);
                return Err(e.into());
            }
        };

        // Send initial message to the new channel
        println!("💬 Sending initial message to channel...");
        let message = CreateMessage::new()
            .content(format!(
                "🎮 **{}** raid scheduled for **{}**!\n\n\
            📅 Date: {}\n\
            👤 Created by: <@{}>\n\n\
            React with ✅ to join this raid!",
                raid_name, selected_weekday, next_date, cmd.user.id
            ))
            .reactions(vec![ReactionType::Unicode("✅".to_string())]);

        let raid_message = match new_channel.send_message(&ctx.http, message).await {
            Ok(msg) => {
                println!("✅ Message sent with ID: {}", msg.id);
                msg
            },
            Err(e) => {
                eprintln!("❌ Failed to send message: {}", e);
                return Err(e.into());
            }
        };

        println!("🔍 Attempting to save raid to database:");
        println!("  Guild ID: {}", guild_id.get() as i64);
        println!("  Channel ID: {}", new_channel.id.get() as i64);
        println!("  Message ID: {}", raid_message.id.get() as i64);
        println!("  Created by: {}", cmd.user.id.get() as i64);
        println!("  Description: {}", raid_name);

        // Convert the date string to DateTime<Utc>
        let scheduled_datetime = match chrono::NaiveDate::parse_from_str(&next_date, "%Y-%m-%d") {
            Ok(date) => {
                println!("✅ Date parsed successfully");
                // Set default time to 20:00 (8 PM) UTC
                date.and_hms_opt(20, 0, 0).unwrap().and_utc()
            },
            Err(e) => {
                eprintln!("❌ Failed to parse date {}: {}", next_date, e);
                // Fallback to tomorrow at 8 PM
                (Utc::now() + Duration::days(1)).date_naive().and_hms_opt(20, 0, 0).unwrap().and_utc()
            }
        };

        println!("  Scheduled for: {}", scheduled_datetime);

        // Save raid to database
        println!("💾 Calling database create_raid function...");
        match crate::db::repo::create_raid(
            pool,
            guild_id.get() as i64,
            new_channel.id.get() as i64,
            scheduled_datetime,
            cmd.user.id.get() as i64,
            raid_name.clone(),
            cmd.user.id.get() as i64,
            raid_message.id.get() as i64,
            false, // is_priority
            String::new(), // priority_list
        ).await {
            Ok(raid) => {
                println!("✅ Raid saved to database with ID: {}", raid.id);
            },
            Err(e) => {
                eprintln!("❌ Failed to save raid to database: {}", e);
                eprintln!("   Error details: {:?}", e);
            }
        }

        // Respond to the command
        println!("📤 Sending response to user...");
        let response = CreateInteractionResponseMessage::new()
            .content(format!(
                "✅ **Raid channel created!**\n\
            📁 Category: `{}`\n\
            📺 Channel: <#{}>\n\
            🎮 Raid: `{}`\n\
            📅 Date: `{}`",
                selected_weekday, new_channel.id, raid_name, next_date
            ));

        match cmd.create_response(&ctx.http, CreateInteractionResponse::Message(response)).await {
            Ok(_) => println!("✅ Response sent successfully"),
            Err(e) => eprintln!("❌ Failed to send response: {}", e),
        }

        println!("🎯 Raid command completed");
        Ok(())
    }

    // Keep your existing helper functions
    pub async fn handle(ctx: &Context, cmd: &CommandInteraction) -> serenity::Result<()> {
        println!("⚠️ Warning: Raid command called without database integration");
        Ok(())
    }
}
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
