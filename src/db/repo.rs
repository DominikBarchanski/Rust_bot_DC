
use crate::db::models::Raid;
use sqlx::PgPool;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Insert a new raid into the database.
pub async fn create_raid(
    pool: &PgPool,
    guild_id: i64,
    channel_id: i64,
    scheduled_for: DateTime<Utc>,
    created_by: i64,
    description: String,
    owner_id: i64,
    message_id: i64,
    is_priority: bool,
    priority_list: String,
) -> anyhow::Result<Raid> {
    let id = Uuid::new_v4();
    let raid = sqlx::query_as!(
        Raid,
        r#"
        INSERT INTO raids (id, guild_id, channel_id, scheduled_for, created_by, description, owner_id, is_priority, is_active, priority_list,message_id)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, true, $9, $10)
        RETURNING id, guild_id, channel_id, scheduled_for, created_by, description, owner_id, is_priority, is_active, priority_list, message_id
        "#,
        id,
        guild_id,
        channel_id,
        scheduled_for,
        created_by,
        description,
        owner_id,
        is_priority,
        priority_list,
        message_id
    )
        .fetch_one(pool)
        .await?;
    Ok(raid)
}

/// Fetch upcoming raids for a guild.
pub async fn get_upcoming_raids(pool: &PgPool, guild_id: i64) -> anyhow::Result<Vec<Raid>> {
    let now = Utc::now();
    let raids = sqlx::query_as!(
        Raid,
        r#"
        SELECT id, guild_id, channel_id, scheduled_for, created_by, description, owner_id, is_priority, is_active, priority_list,message_id
        FROM raids
        WHERE guild_id = $1 AND scheduled_for > $2
        ORDER BY scheduled_for ASC
        "#,
        guild_id,
        now
    )
        .fetch_all(pool)
        .await?;
    Ok(raids)
}