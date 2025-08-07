mod commands;
mod handlers;
mod modal;

use dotenvy::dotenv;
use serenity::all::{Client, GatewayIntents};
use tokio_postgres::NoTls;
use std::env;

#[tokio::main]
async fn main() {
    dotenv().ok();

    let token = env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN not set");
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL not set");

    // Setup database connection
    let (client_pg, connection) = tokio_postgres::connect(&db_url, NoTls)
        .await
        .expect("Failed to connect to DB");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("DB connection error: {}", e);
        }
    });

    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT; // Needed for reactions

    let handler = handlers::raid_handler::Handler {
        db_client: client_pg,
    };

    let mut client = Client::builder(&token, intents)
        .event_handler(handler)
        .await
        .expect("Error creating Discord client");

    if let Err(err) = client.start().await {
        eprintln!("Client error: {:?}", err);
    }
}