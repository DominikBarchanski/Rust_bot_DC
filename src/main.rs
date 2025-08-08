mod db;
mod commands;
mod handlers;
mod ui;
mod utils;
mod tasks;

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

    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_MESSAGES;
    let handler = Handler::new(pool);

    let mut client = Client::builder(&token, intents)
        .event_handler(handler)
        .await?;

    client.start().await?;
    Ok(())
}
