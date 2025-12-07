mod db;
mod commands;
mod handlers;
mod ui;
mod utils;
mod tasks;
mod redis_ext;
mod queue;

use crate::handlers::Handler;
use dotenvy::dotenv;
use serenity::all::{Client, GatewayIntents};
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    tracing_subscriber::fmt::init();

    let token = env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN not set");
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL not set");

    let pool = db::init_pool(&db_url).await?;

    // Redis (required for raid list registry)
    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    let redis_client = redis::Client::open(redis_url)?;
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MESSAGE_REACTIONS;
    let handler = Handler::new(pool, redis_client);

    let mut client = Client::builder(&token, intents)
        .event_handler(handler)
        .await?;

    client.start().await?;
    Ok(())
}
