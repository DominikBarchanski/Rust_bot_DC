pub mod components;

use serenity::all::{Context, EventHandler, Interaction, Ready};
use serenity::async_trait;
use sqlx::PgPool;

pub struct Handler {
    pool: PgPool,
}

impl Handler {
    pub fn new(pool: PgPool) -> Self { Self { pool } }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
        {
            let mut data = ctx.data.write().await;
            data.insert::<DbKey>(self.pool.clone());
        }

        // (Optional) register slash commands
        if let Err(e) = crate::commands::register_commands(&ctx).await {
            eprintln!("Failed to register commands: {e}");
        }

        // Restore all scheduled jobs after restart (non-blocking)
        let http = ctx.http.clone();
        let pool = self.pool.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::tasks::restore_schedules(http, pool).await {
                eprintln!("restore_schedules failed: {e:#}");
            }
        });
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        use serenity::all::Interaction::*;
        match interaction {
            Command(cmd) => {
                if let Err(e) = crate::commands::raid::handle(&ctx, &cmd).await {
                    eprintln!("command error: {e}");
                }
            }
            Component(comp) => {
                if let Err(e) = components::handle_component(&ctx, &comp).await {
                    eprintln!("component error: {e}");
                }
            }
            _ => {}
        }
    }
}

/* Context data access */
use serenity::prelude::TypeMapKey;
struct DbKey;
impl TypeMapKey for DbKey { type Value = PgPool; }
pub async fn pool_from_ctx(ctx: &Context) -> anyhow::Result<PgPool> {
    let data = ctx.data.read().await;
    Ok(data.get::<DbKey>().cloned().expect("PgPool missing"))
}
