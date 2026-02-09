pub mod components;

use serenity::all::{Context, EventHandler, Interaction, Ready, GuildChannel, Message};
use serenity::async_trait;
use sqlx::PgPool;
use std::sync::Arc;
use crate::queue;

pub struct Handler {
    pool: PgPool,
    redis: redis::Client,
}

impl Handler {
    pub fn new(pool: PgPool, redis: redis::Client) -> Self { Self { pool, redis } }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
        {
            let mut data = ctx.data.write().await;
            data.insert::<DbKey>(self.pool.clone());
            data.insert::<RedisKey>(self.redis.clone());
        }

        // (Optional) register slash commands
        if let Err(e) = crate::commands::register_commands(&ctx).await {
            eprintln!("Failed to register commands: {e}");
        }

        // Restore all scheduled jobs after restart (non-blocking)
        let http = ctx.http.clone();
        let pool = self.pool.clone();
        let redis = self.redis.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::tasks::restore_schedules(http, pool, redis).await {
                eprintln!("restore_schedules failed: {e:#}");
            }
        });

        // Periodic refresh of guild raid lists every 5 minutes + initial kick-off
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            use tokio::time::{interval, Duration};
            // initial pass right away
            if let Err(e) = crate::commands::raid::refresh_all_guild_lists_if_any(&ctx2).await {
                eprintln!("initial list refresh failed: {e:#}");
            }
            let mut tick = interval(Duration::from_secs(300));
            loop {
                tick.tick().await;
                if let Err(e) = crate::commands::raid::refresh_all_guild_lists_if_any(&ctx2).await {
                    eprintln!("periodic list refresh failed: {e:#}");
                }
            }
        });

        // Start Redis consumer for DB-write events
        let ctx_consumer = ctx.clone();
        let pool_consumer = self.pool.clone();
        let redis_consumer = self.redis.clone();
        tokio::spawn(async move {
            queue::run_consumer(ctx_consumer, pool_consumer, redis_consumer).await;
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
    async fn channel_delete(&self, _ctx: Context, channel: GuildChannel, _messages: Option<Vec<Message>>) {
        let ch_id = channel.id.get() as i64;
        if let Err(e) = crate::db::repo::inactive_raid_after_delete_channel(&self.pool, ch_id).await {
            eprintln!("inactive_raid_after_delete_channel failed: {e:#}");
        }

        // If this channel hosted a guild list, purge mapping (DB + Redis)
        match crate::db::repo::get_guild_raid_list_by_channel(&self.pool, ch_id).await {
            Ok(Some(row)) => {
                if let Err(e) = crate::db::repo::delete_guild_raid_list_by_guild(&self.pool, row.guild_id).await {
                    eprintln!("delete_guild_raid_list_by_guild failed: {e:#}");
                }
                if let Err(e) = crate::redis_ext::del_guild_list(&self.redis, row.guild_id as u64).await {
                    eprintln!("redis del_guild_list failed: {e:#}");
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("get_guild_raid_list_by_channel failed: {e:#}"),
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

struct RedisKey;
impl TypeMapKey for RedisKey { type Value = redis::Client; }
pub async fn redis_from_ctx(ctx: &Context) -> anyhow::Result<redis::Client> {
    let data = ctx.data.read().await;
    Ok(data.get::<RedisKey>().cloned().expect("Redis client missing"))
}
