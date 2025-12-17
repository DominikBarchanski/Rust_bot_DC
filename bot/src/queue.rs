use anyhow::Context;
use redis::{AsyncCommands, aio::MultiplexedConnection, Value};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::db::repo;
use serenity::all::{Context as DiscordContext, GuildId, UserId};

const STREAM_KEY: &str = "raid_events";
const GROUP_NAME: &str = "raid_bot";

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RaidEvent {
    Join {
        raid_id: Uuid,
        guild_id: i64,
        user_id: i64,
        joined_as: String,
        main_now: bool,
        tag_suffix: String,
        is_alt: bool,
    },
    LeaveAll {
        raid_id: Uuid,
        guild_id: i64,
        user_id: i64,
    },
    LeaveAlts {
        raid_id: Uuid,
        guild_id: i64,
        user_id: i64,
    },
    AddSp {
        raid_id: Uuid,
        guild_id: i64,
        user_id: i64,
        sp: String,
    },
    ChangeSp {
        raid_id: Uuid,
        guild_id: i64,
        user_id: i64,
        sp: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AckPayload {
    pub ok: bool,
    pub removed_main: Option<u64>,
    pub removed_alts: Option<u64>,
}

async fn ensure_group(conn: &mut MultiplexedConnection) -> anyhow::Result<()> {
    // XGROUP CREATE <stream> <group> $ MKSTREAM
    let _: Option<String> = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(STREAM_KEY)
        .arg(GROUP_NAME)
        .arg("$")
        .arg("MKSTREAM")
        .query_async(conn)
        .await
        .ok();
    Ok(())
}

pub async fn publish(redis: &redis::Client, ev: &RaidEvent) -> anyhow::Result<String> {
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .context("redis connect publish")?;
    ensure_group(&mut conn).await.ok();
    let cid = Uuid::new_v4().to_string();
    let payload = serde_json::to_string(ev)?;
    // XADD raid_events * cid <cid> payload <json>
    let _: String = redis::cmd("XADD")
        .arg(STREAM_KEY)
        .arg("*")
        .arg("cid")
        .arg(&cid)
        .arg("payload")
        .arg(&payload)
        .query_async(&mut conn)
        .await
        .context("redis XADD publish")?;
    Ok(cid)
}

pub async fn wait_for_ack(redis: &redis::Client, corr_id: &str, timeout_ms: u64) -> anyhow::Result<Option<AckPayload>> {
    let mut conn = redis
        .get_multiplexed_async_connection()
        .await
        .context("redis connect wait_for_ack")?;
    let key = format!("raid_ack:{corr_id}");
    let mut slept = 0u64;
    let step = 50u64;
    loop {
        let v: Option<String> = conn.get(&key).await.ok();
        if let Some(s) = v { return Ok(serde_json::from_str(&s).ok()); }
        if slept >= timeout_ms { return Ok(None); }
        tokio::time::sleep(std::time::Duration::from_millis(step)).await;
        slept += step;
    }
}

async fn set_ack(conn: &mut MultiplexedConnection, corr_id: &str, ack: &AckPayload) -> anyhow::Result<()> {
    let key = format!("raid_ack:{corr_id}");
    let payload = serde_json::to_string(ack)?;
    // SETEX key ttl value
    let _: () = redis::cmd("SETEX")
        .arg(&key)
        .arg(5) // seconds
        .arg(&payload)
        .query_async(conn)
        .await
        .context("redis SETEX ack")?;
    Ok(())
}

pub async fn run_consumer(ctx: DiscordContext, pool: sqlx::PgPool, redis: redis::Client) {
    use redis::streams::{StreamReadOptions, StreamReadReply};
    let consumer = format!("bot-{}", std::process::id());
    let mut conn = match redis.get_multiplexed_async_connection().await {
        Ok(c) => c,
        Err(e) => { eprintln!("redis connect consumer error: {e:#}"); return; }
    };
    let _ = ensure_group(&mut conn).await;

    loop {
        let opts = StreamReadOptions::default()
            .group(GROUP_NAME, &consumer)
            .count(20)
            .block(2000);
        let reply: Result<StreamReadReply, _> = redis::cmd("XREADGROUP")
            .arg("GROUP").arg(GROUP_NAME).arg(&consumer)
            .arg("COUNT").arg(20)
            .arg("BLOCK").arg(2000)
            .arg("STREAMS").arg(STREAM_KEY)
            .arg(">")
            .query_async(&mut conn)
            .await;

        let Ok(rep) = reply else { continue; };
        for stream in rep.keys {
            for id in stream.ids {
                // Extract fields into map
                let mut cid: Option<String> = None;
                let mut payload: Option<String> = None;
                for (k, v) in id.map.iter() {
                    if k == "cid" { cid = val_to_string(v); }
                    if k == "payload" { payload = val_to_string(v); }
                }
                let Some(corr_id) = cid else { continue; };
                let Some(payload_s) = payload else { continue; };
                let evt: Result<RaidEvent, _> = serde_json::from_str(&payload_s);
                let ack_res = match evt {
                    Ok(RaidEvent::Join { raid_id, guild_id, user_id, joined_as, main_now, tag_suffix, is_alt }) => {
                        handle_join(&ctx, &pool, raid_id, guild_id, user_id, joined_as, main_now, tag_suffix, is_alt).await
                    }
                    Ok(RaidEvent::LeaveAll { raid_id, guild_id, user_id }) => {
                        handle_leave_all(&ctx, &pool, raid_id, guild_id, user_id).await
                    }
                    Ok(RaidEvent::LeaveAlts { raid_id, guild_id, user_id }) => {
                        handle_leave_alts(&ctx, &pool, raid_id, guild_id, user_id).await
                    }
                    Ok(RaidEvent::AddSp { raid_id, guild_id, user_id, sp }) => {
                        handle_add_sp(&ctx, &pool, raid_id, guild_id, user_id, sp).await
                    }
                    Ok(RaidEvent::ChangeSp { raid_id, guild_id, user_id, sp }) => {
                        handle_change_sp(&ctx, &pool, raid_id, guild_id, user_id, sp).await
                    }
                    Err(e) => { eprintln!("queue: bad payload: {e:#}"); Ok(AckPayload { ok: false, removed_main: None, removed_alts: None }) }
                };

                // write ack regardless of result
                let _ = set_ack(&mut conn, &corr_id, &ack_res.unwrap_or(AckPayload { ok: false, removed_main: None, removed_alts: None })).await;
                // acknowledge stream item
                let _: () = redis::cmd("XACK")
                    .arg(STREAM_KEY)
                    .arg(GROUP_NAME)
                    .arg(&id.id)
                    .query_async(&mut conn)
                    .await
                    .unwrap_or(());
            }
        }
    }
}

fn val_to_string(v: &Value) -> Option<String> {
    match v {
        Value::Data(bytes) => String::from_utf8(bytes.clone()).ok(),
        Value::Int(i) => Some(i.to_string()),
        Value::Status(s) => Some(s.clone()),
        Value::Okay => Some("OK".to_string()),
        Value::Nil => None,
        Value::Bulk(items) => {
            // join first element if it's Data/Status
            if let Some(first) = items.get(0) {
                val_to_string(first)
            } else {
                None
            }
        }
    }
}

async fn handle_join(
    ctx: &DiscordContext,
    pool: &sqlx::PgPool,
    raid_id: Uuid,
    guild_id: i64,
    user_id: i64,
    joined_as: String,
    main_now: bool,
    tag_suffix: String,
    is_alt: bool,
) -> anyhow::Result<AckPayload> {
    // Upsert main or insert alt
    if is_alt {
        let _ = repo::insert_alt(pool, raid_id, user_id, joined_as, main_now, tag_suffix).await?;
    } else {
        let _ = repo::insert_or_replace_main(pool, raid_id, user_id, joined_as, main_now, tag_suffix).await?;
    }

    // After join, run promotion
    let raid = repo::get_raid(pool, raid_id).await?;
    let now = chrono::Utc::now();
    let mut exclude_ids: Vec<i64> = Vec::new();
    let mut priority_user_ids: Vec<i64> = Vec::new();
    let gid = GuildId::new(guild_id as u64);
    if let Ok(roles_map) = gid.roles(&ctx.http).await {
        let reserve_role_name = std::env::var("RESERVE_ROLE_NAME").unwrap_or_else(|_| "reserve".to_string());
        let parts_for_check = repo::list_participants(pool, raid_id).await?;
        // Priority role IDs configured on the raid (array of BIGINT)
        let pr_ids: Vec<i64> = raid
            .priority_role_id
            .as_ref()
            .map(|v| v.clone())
            .unwrap_or_default();
        let pr_set: std::collections::HashSet<u64> = pr_ids.iter().map(|x| *x as u64).collect();

        for p in &parts_for_check {
            if let Ok(member) = gid.member(&ctx.http, UserId::new(p.user_id as u64)).await {
                // exclude: users with reserve role name
                let has_reserve = member.roles.iter().any(|rid| {
                    roles_map.get(rid).map_or(false, |r| r.name.eq_ignore_ascii_case(&reserve_role_name))
                });
                if has_reserve { exclude_ids.push(p.user_id); }

                // priority: users that have any of the configured priority role IDs
                if !pr_set.is_empty() {
                    let has_priority = member.roles.iter().any(|rid| pr_set.contains(&rid.get()));
                    if has_priority { priority_user_ids.push(p.user_id); }
                }
            }
        }
    }

    // Choose promotion strategy
    // During active priority window (or indefinite when is_priority=true and no until): promote ONLY users with priority roles.
    let active_priority = raid.is_priority && raid.priority_until.map(|u| now < u).unwrap_or(true);
    if active_priority {
        let _ = repo::promote_reserves_with_priority_excluding(
            pool, raid_id, raid.max_players, raid.max_alts, &priority_user_ids, &exclude_ids
        ).await;
    } else {
        // default ordering
        let _ = repo::promote_reserves_with_alt_limits_excluding(
            pool, raid_id, raid.max_players, raid.max_alts, &exclude_ids
        ).await;
    }

    // Force refresh consolidated list immediately
    let _ = crate::commands::raid::force_refresh_guild_raid_list(ctx, guild_id as u64).await;
    Ok(AckPayload { ok: true, removed_main: None, removed_alts: None })
}

async fn handle_leave_all(
    ctx: &DiscordContext,
    pool: &sqlx::PgPool,
    raid_id: Uuid,
    guild_id: i64,
    user_id: i64,
) -> anyhow::Result<AckPayload> {
    let removed_main = repo::remove_participant(pool, raid_id, user_id).await.unwrap_or(0);
    let removed_alts = repo::remove_user_alts(pool, raid_id, user_id).await.unwrap_or(0);

    // Promote immediately after a leave.
    let raid = repo::get_raid(pool, raid_id).await?;
    let now = chrono::Utc::now();
    let mut exclude_ids: Vec<i64> = Vec::new();
    let mut priority_user_ids: Vec<i64> = Vec::new();
    let gid = GuildId::new(guild_id as u64);
    if let Ok(roles_map) = gid.roles(&ctx.http).await {
        let reserve_role_name = std::env::var("RESERVE_ROLE_NAME").unwrap_or_else(|_| "reserve".to_string());
        let parts_for_check = repo::list_participants(pool, raid_id).await?;
        // Priority role IDs configured on the raid (array of BIGINT)
        let pr_ids: Vec<i64> = raid
            .priority_role_id
            .as_ref()
            .map(|v| v.clone())
            .unwrap_or_default();
        let pr_set: std::collections::HashSet<u64> = pr_ids.iter().map(|x| *x as u64).collect();

        for p in &parts_for_check {
            if let Ok(member) = gid.member(&ctx.http, UserId::new(p.user_id as u64)).await {
                let has_reserve = member.roles.iter().any(|rid| {
                    roles_map.get(rid).map_or(false, |r| r.name.eq_ignore_ascii_case(&reserve_role_name))
                });
                if has_reserve { exclude_ids.push(p.user_id); }

                if !pr_set.is_empty() {
                    let has_priority = member.roles.iter().any(|rid| pr_set.contains(&rid.get()));
                    if has_priority { priority_user_ids.push(p.user_id); }
                }
            }
        }
    }

    // During active priority window (or indefinite when is_priority=true and no until): promote ONLY users with priority roles.
    let active_priority = raid.is_priority && raid.priority_until.map(|u| now < u).unwrap_or(true);
    if active_priority {
        let _ = repo::promote_reserves_with_priority_excluding(
            pool, raid_id, raid.max_players, raid.max_alts, &priority_user_ids, &exclude_ids
        ).await;
    } else {
        let _ = repo::promote_reserves_with_alt_limits_excluding(
            pool, raid_id, raid.max_players, raid.max_alts, &exclude_ids
        ).await;
    }

    let _ = crate::commands::raid::force_refresh_guild_raid_list(ctx, guild_id as u64).await;
    Ok(AckPayload { ok: true, removed_main: Some(removed_main), removed_alts: Some(removed_alts) })
}

async fn handle_leave_alts(
    ctx: &DiscordContext,
    pool: &sqlx::PgPool,
    raid_id: Uuid,
    guild_id: i64,
    user_id: i64,
) -> anyhow::Result<AckPayload> {
    let removed = repo::remove_user_alts(pool, raid_id, user_id).await.unwrap_or(0);
    // Consolidated list isn't affected by alt-only changes in count of mains, but keep it consistent anyway
    crate::commands::raid::trigger_refresh(ctx, guild_id as u64).await;
    Ok(AckPayload { ok: true, removed_main: None, removed_alts: Some(removed) })
}

async fn handle_add_sp(
    ctx: &DiscordContext,
    pool: &sqlx::PgPool,
    raid_id: Uuid,
    guild_id: i64,
    user_id: i64,
    sp: String,
) -> anyhow::Result<AckPayload> {
    // Append SP to user's main row
    repo::append_extra_sp(pool, raid_id, user_id, &sp).await?;
    // Trigger both immediate and backup refresh of consolidated list (embed is refreshed by the interaction handler)
    let _ = crate::commands::raid::force_refresh_guild_raid_list(ctx, guild_id as u64).await;
    crate::commands::raid::trigger_refresh(ctx, guild_id as u64).await;
    Ok(AckPayload { ok: true, removed_main: None, removed_alts: None })
}

async fn handle_change_sp(
    ctx: &DiscordContext,
    pool: &sqlx::PgPool,
    raid_id: Uuid,
    guild_id: i64,
    user_id: i64,
    sp: String,
) -> anyhow::Result<AckPayload> {
    // Read current main row to get class part
    if let Some(main) = repo::get_user_main_row(pool, raid_id, user_id).await? {
        let class_part = main.joined_as.split('/').next().map(|s| s.trim().to_string()).unwrap_or_else(|| "MSW".to_string());
        repo::set_active_sp(pool, raid_id, user_id, &class_part, &sp).await?;
        let _ = crate::commands::raid::force_refresh_guild_raid_list(ctx, guild_id as u64).await;
        crate::commands::raid::trigger_refresh(ctx, guild_id as u64).await;
        Ok(AckPayload { ok: true, removed_main: None, removed_alts: None })
    } else {
        Ok(AckPayload { ok: false, removed_main: None, removed_alts: None })
    }
}
