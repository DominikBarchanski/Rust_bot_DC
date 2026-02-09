use anyhow::Context;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const KEY_PREFIX: &str = "guild_raid_list:";
const REMINDER_15M_PREFIX: &str = "raid_reminder_15m:";

#[derive(Debug, Serialize, Deserialize)]
struct GuildListRecord {
    channel: u64,
    ids: Vec<u64>,
}

fn key_for(guild_id: u64) -> String { format!("{}{}", KEY_PREFIX, guild_id) }

fn reminder_15m_key(raid_id: Uuid) -> String { format!("{}{}", REMINDER_15M_PREFIX, raid_id) }

pub async fn get_guild_list(client: &redis::Client, guild_id: u64) -> anyhow::Result<Option<(u64, Vec<u64>)>> {
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .context("redis connect")?;
    let key = key_for(guild_id);
    let v: Option<String> = conn.get(key).await.context("redis GET guild list")?;
    Ok(match v {
        Some(s) => {
            let rec: GuildListRecord = serde_json::from_str(&s).context("parse guild list json")?;
            Some((rec.channel, rec.ids))
        }
        None => None,
    })
}

pub async fn set_guild_list(client: &redis::Client, guild_id: u64, channel_id: u64, message_ids: &[u64]) -> anyhow::Result<()> {
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .context("redis connect")?;
    let key = key_for(guild_id);
    let rec = GuildListRecord { channel: channel_id, ids: message_ids.to_vec() };
    let payload = serde_json::to_string(&rec)?;
    // set with TTL 24h; DB is the durable store anyway
    let _: () = redis::cmd("SET")
        .arg(&[key.as_str(), payload.as_str(), "EX", "86400"]) // 24h
        .query_async(&mut conn)
        .await
        .context("redis SET guild list")?;
    Ok(())
}

pub async fn del_guild_list(client: &redis::Client, guild_id: u64) -> anyhow::Result<()> {
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .context("redis connect")?;
    let key = key_for(guild_id);
    let _: () = conn.del(key).await.context("redis DEL guild list")?;
    Ok(())
}

pub async fn claim_raid_reminder_15m(client: &redis::Client, raid_id: Uuid) -> anyhow::Result<bool> {
    let mut conn = client
        .get_multiplexed_async_connection()
        .await
        .context("redis connect")?;
    let key = reminder_15m_key(raid_id);
    let ttl_seconds = 60 * 60 * 48;
    let res: Option<String> = redis::cmd("SET")
        .arg(&key)
        .arg("1")
        .arg("NX")
        .arg("EX")
        .arg(ttl_seconds)
        .query_async(&mut conn)
        .await
        .context("redis SETNX reminder")?;
    Ok(res.is_some())
}
