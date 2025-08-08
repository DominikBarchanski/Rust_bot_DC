use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::types::Json;
use uuid::Uuid;

#[derive(sqlx::FromRow, Serialize, Deserialize, Debug, Clone)]
pub struct Raid {
    pub id: Uuid,
    pub guild_id: i64,
    pub channel_id: i64,
    pub message_id: i64,
    pub scheduled_for: DateTime<Utc>,
    pub created_by: i64,
    pub owner_id: i64,
    pub description: String,
    pub is_priority: bool,
    pub is_active: bool,
    pub priority_list: Json<Vec<i64>>,

    pub raid_name: String,
    pub max_players: i32,
    pub allow_alts: bool,
    pub max_alts: i32,
    pub priority_role_id: Option<i64>,
    pub priority_until: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow, Serialize, Deserialize, Debug, Clone)]
pub struct RaidParticipant {
    pub id: Uuid,
    pub raid_id: Uuid,
    pub user_id: i64,
    pub is_main: bool,
    pub joined_as: String,
    pub is_reserve: bool,
    pub joined_at: DateTime<Utc>,
    pub is_alt: bool,
    pub tag_suffix: String,
}
