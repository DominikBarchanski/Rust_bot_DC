use std::sync::Arc;
use tokio::time::{sleep_until, Duration, Instant};
use std::collections::HashMap;
use chrono_tz::Europe::Warsaw;
use serenity::http::Http;
use serenity::all::ChannelId;
use sqlx::PgPool;
use uuid::Uuid;
use chrono::Duration as CDuration;

use crate::db::repo;

pub fn schedule_priority_promotion(
    http: Arc<Http>,
    pool: PgPool,
    raid_id: Uuid,
    guild_id: i64,   // <— use this
    channel_id: i64,
    message_id: i64,
    run_at: chrono::DateTime<chrono::Utc>,
) {
    let wait = (run_at - chrono::Utc::now()).to_std().unwrap_or(Duration::from_secs(0));
    let when = Instant::now() + wait;
    tokio::spawn(async move {
        sleep_until(when).await;
        let _ = promote_and_refresh(&http, &pool, raid_id, guild_id, channel_id, message_id).await;
    });
}


pub fn schedule_auto_delete(
    http: Arc<Http>,
    pool: PgPool,                 // <— NOWE
    _raid_id: Uuid,                // (zostaw jeśli używasz gdzie indziej)
    channel_id: i64,
    run_at: chrono::DateTime<chrono::Utc>,
) {
    let wait = (run_at - chrono::Utc::now()).to_std().unwrap_or(Duration::from_secs(0));
    let when = Instant::now() + wait;
    tokio::spawn(async move {
        sleep_until(when).await;
        if ChannelId::new(channel_id as u64).delete(&http).await.is_ok() {
            let _ = crate::db::repo::inactive_raid_after_delete_channel(&pool, channel_id).await;
        }
    });
}

async fn promote_and_refresh(
    http: &Http,
    pool: &PgPool,
    raid_id: Uuid,
    guild_id: i64,
    channel_id: i64,
    message_id: i64,
) -> anyhow::Result<()> {
    use serenity::all::{GuildId, UserId};
    let raid = repo::get_raid(pool, raid_id).await?;

    // Run only AFTER the priority window ended
    if let Some(until) = raid.priority_until {
        if chrono::Utc::now() < until {
            return Ok(()); // too early, skip
        }
    }

    // Exclude users who currently have the RESERVE_ROLE_NAME role
    let mut exclude_ids: Vec<i64> = Vec::new();
    let gid = GuildId::new(guild_id as u64);
    if let Ok(roles_map) = gid.roles(http).await {
        let reserve_role_name = std::env::var("RESERVE_ROLE_NAME").unwrap_or_else(|_| "reserve".to_string());
        let parts_for_check = repo::list_participants(pool, raid_id).await?;
        for p in &parts_for_check {
            if let Ok(member) = gid.member(http, UserId::new(p.user_id as u64)).await {
                let has_reserve = member.roles.iter().any(|rid| {
                    roles_map.get(rid).map_or(false, |r| r.name.eq_ignore_ascii_case(&reserve_role_name))
                });
                if has_reserve {
                    exclude_ids.push(p.user_id);
                }
            }
        }
    }

    // Promote with exclusions
    let _ = repo::promote_reserves_with_alt_limits_excluding(
        pool, raid_id, raid.max_players, raid.max_alts, &exclude_ids
    ).await?;

    // Refresh embed
    let raid = repo::get_raid(pool, raid_id).await?;
    let parts = repo::list_participants(pool, raid_id).await?;
    let embed = crate::ui::embeds::render_raid_embed_plain(&raid, &parts);

    ChannelId::new(channel_id as u64)
        .edit_message(http, message_id as u64, serenity::builder::EditMessage::new().embed(embed))
        .await?;
    Ok(())
}

/// Spawn one timer: at (scheduled_for - 15m) DM **current** participants (mains + reserves)
pub fn schedule_raid_15m_reminder(
    http: Arc<Http>,
    pool: PgPool,
    redis: redis::Client,
    raid_id: uuid::Uuid,
    scheduled_for_utc: chrono::DateTime<chrono::Utc>,
) {
    let wait = (scheduled_for_utc - chrono::Utc::now()).to_std().unwrap_or(Duration::from_secs(0));
    let when_inst = Instant::now() + wait;

    tokio::spawn(async move {
        sleep_until(when_inst).await;

        // Resolve raid + participants at send time (so it's always up-to-date)
        let raid = match crate::db::repo::get_raid(&pool, raid_id).await {
            Ok(r) => r, Err(_) => return,
        };

        // If the raid was cancelled in the meantime, do not send DMs
        if !raid.is_active {
            return;
        }

        let claimed = match crate::redis_ext::claim_raid_reminder_15m(&redis, raid_id).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("claim_raid_reminder_15m failed: {e:#}");
                true
            }
        };
        if !claimed {
            return;
        }

        let parts = match crate::db::repo::list_participants(&pool, raid_id).await {
            Ok(v) => v, Err(_) => return,
        };

        let when_local = raid.scheduled_for.with_timezone(&Warsaw).format("%Y-%m-%d %H:%M %Z");
        let chan_mention = format!("<#{}>", raid.channel_id as u64);

        // unique per user: prefer MAIN if they have any main row
        let mut main_any_by_user: HashMap<i64, bool> = HashMap::new();
        for p in parts {
            main_any_by_user
                .entry(p.user_id)
                .and_modify(|m| *m = *m || p.is_main)
                .or_insert(p.is_main);
        }

        for (uid, main_any) in main_any_by_user {
            let status = if main_any { "MAIN" } else { "RESERVE" };
            let msg = format!(
                "⏰ Reminder: **{}** starts at **{}**.\nChannel: {}\nYour status: **{}**",
                raid.raid_name, when_local, chan_mention, status
            );
            crate::utils::dm_user(&http, uid as u64, msg).await;
        }
    });
}

pub async fn restore_schedules(
    http: Arc<Http>,
    pool: PgPool,
    redis: redis::Client,
) -> anyhow::Result<()> {
    // Fetch all active raids we might need to handle
    let raids = repo::list_active_raids_for_restore(&pool).await?;

    for r in raids {
        // 3a) Priority promotion at priority_until
        if let Some(until) = r.priority_until {
            if chrono::Utc::now() < until {
                schedule_priority_promotion(
                    http.clone(), pool.clone(), r.id, r.guild_id, r.channel_id, r.message_id, until
                );
            } else {
                // We missed it while offline → run once now
                let _ = promote_and_refresh(&http, &pool, r.id, r.guild_id, r.channel_id, r.message_id).await;
            }
        }

        // 3b) 15-minute reminder
        let reminder_at = r.scheduled_for - CDuration::minutes(15);
        if chrono::Utc::now() < reminder_at {
            schedule_raid_15m_reminder(http.clone(), pool.clone(), redis.clone(), r.id, reminder_at);
        } else if chrono::Utc::now() < r.scheduled_for {
            // missed but raid not started yet → send immediately
            schedule_raid_15m_reminder(http.clone(), pool.clone(), redis.clone(), r.id, chrono::Utc::now());
        }

        // 3c) Auto-delete (recompute from description like at creation)
        let (_desc_clean, dur_h) = crate::utils::extract_duration_hours(&r.description);
        let duration_for_schedule: i64 = dur_h.ceil() as i64;
        let delete_at = r.scheduled_for
            + CDuration::hours(duration_for_schedule)
            + CDuration::minutes(20);

        if chrono::Utc::now() < delete_at {
            schedule_auto_delete(http.clone(),pool.clone(), r.id, r.channel_id, delete_at);
        } else {
            let _ = ChannelId::new(r.channel_id as u64).delete(&http).await;
            let _ = crate::db::repo::inactive_raid_after_delete_channel(&pool, r.channel_id).await;
        }
    }

    Ok(())
}
