use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serenity::all::UserId;
use uuid::Uuid;

/// Represents a scheduled raid.
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
    pub priority_list:String
}

#[derive(sqlx::FromRow, Serialize, Deserialize, Debug, Clone)]
pub struct RaidParticipant {
    pub id: Uuid,
    pub raid_id: i64,
    pub user_id: UserId,
    pub is_main: bool,
    pub joined_as: String,
    pub is_reserve: bool

}