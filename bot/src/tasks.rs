use std::sync::Arc;
use tokio::time::{sleep_until, Duration, Instant};
use serenity::http::Http;
use serenity::all::ChannelId;
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::repo;

pub fn schedule_priority_promotion(
    http: Arc<Http>,
    pool: PgPool,
    raid_id: Uuid,
    _guild_id: i64,
    channel_id: i64,
    message_id: i64,
    run_at: chrono::DateTime<chrono::Utc>,
) {
    let wait = (run_at - chrono::Utc::now()).to_std().unwrap_or(Duration::from_secs(0));
    let when = Instant::now() + wait;
    tokio::spawn(async move {
        sleep_until(when).await;
        let _ = promote_and_refresh(&http, &pool, raid_id, channel_id, message_id).await;
    });
}

pub fn schedule_auto_delete(
    http: Arc<Http>,
    _raid_id: Uuid,
    channel_id: i64,
    run_at: chrono::DateTime<chrono::Utc>,
) {
    let wait = (run_at - chrono::Utc::now()).to_std().unwrap_or(Duration::from_secs(0));
    let when = Instant::now() + wait;
    tokio::spawn(async move {
        sleep_until(when).await;
        let _ = ChannelId::new(channel_id as u64).delete(&http).await;
    });
}

async fn promote_and_refresh(
    http: &Http,
    pool: &PgPool,
    raid_id: Uuid,
    channel_id: i64,
    message_id: i64,
) -> anyhow::Result<()> {
    let raid = repo::get_raid(pool, raid_id).await?;
    let _ = repo::promote_reserves_with_alt_limits(pool, raid_id, raid.max_players, raid.max_alts).await?;

    let raid = repo::get_raid(pool, raid_id).await?;
    let parts = repo::list_participants(pool, raid_id).await?;
    let embed = crate::ui::embeds::render_raid_embed_plain(&raid, &parts);

    ChannelId::new(channel_id as u64)
        .edit_message(http, message_id as u64, serenity::builder::EditMessage::new().embed(embed))
        .await?;
    Ok(())
}
