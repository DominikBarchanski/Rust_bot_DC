mod db;
mod commands;
mod ui;
// mod raid_handler;
use dotenvy::dotenv;
use serenity::{Client, all::GatewayIntents};
use std::env;
use tracing_subscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load environment vars from .env
    dotenv().ok();
    tracing_subscriber::fmt::init();

    // Read configuration from environment
    let token = env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN not set");
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL not set");

    // Initialize Postgres pool
    let pool = db::init_pool(&db_url).await?;

    // Define the intents your bot needs
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    // Create Discord client
    let handler = ui::Handler::new(pool.clone());
    let mut client = Client::builder(&token, intents)
        .event_handler(handler)
        .await?;

    // Start the bot
    client.start().await?;
    Ok(())
}