use uuid::Uuid;
use chrono::{NaiveDate, NaiveTime, NaiveDateTime, Utc, TimeZone};
use chrono::Datelike;
use crate::handlers::pool_from_ctx;
use chrono_tz::Europe::Warsaw;
use serenity::all::{Context, Emoji, UserId,Http,GuildId};
use serenity::prelude::Mentionable;
use once_cell::sync::Lazy;
use dashmap::DashMap;
use std::time::Duration as StdDuration;
use tokio::time::Instant;
use regex::Regex;

pub const ORGANISER_ROLE_NAME: &str = "raid_organiser";
static NAME_CACHE: Lazy<DashMap<u64, (String, Instant)>> = Lazy::new(DashMap::new);
const NAME_TTL: StdDuration = StdDuration::from_secs(60 * 30); // 30 min


pub fn from_user_id(id: serenity::all::UserId) -> i64 { id.get() as i64 }
fn cache_get(user_id: u64) -> Option<String> {
    if let Some(entry) = NAME_CACHE.get(&user_id) {
        let (name, when) = entry.value();
        if when.elapsed() < NAME_TTL {
            return Some(name.clone());
        }
    }
    None
}

fn cache_put(user_id: u64, name: String) {
    NAME_CACHE.insert(user_id, (name, Instant::now()));
}

/// Best-effort display name (async, with HTTP fallback).
/// If `guild_id` is Some, prefers **guild nickname**; otherwise falls back to global display name / username.
/// Will cache names for a short TTL.
pub async fn user_name_best(ctx: &Context, guild_id: Option<u64>, user_id: i64) -> String {
    let uid64 = user_id as u64;

    // 0) fast path: local TTL cache
    if let Some(n) = cache_get(uid64) {
        return n;
    }

    // 1) if we know the guild, try cache -> HTTP member
    if let Some(gid_u64) = guild_id {
        let gid = GuildId::new(gid_u64);
        if let Some(g) = ctx.cache.guild(gid) {
            if let Some(m) = g.members.get(&UserId::new(uid64)) {
                let name = m.nick.clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| m.user.global_name.clone().filter(|s| !s.is_empty()))
                    .unwrap_or_else(|| m.user.name.clone());
                cache_put(uid64, name.clone());
                return name;
            }
        }
        // Not cached → fetch the member (gets nickname if present)
        if let Ok(m) = gid.member(&ctx.http, UserId::new(uid64)).await {
            let name = m.nick
                .filter(|s| !s.is_empty())
                .or_else(|| m.user.global_name.clone().filter(|s| !s.is_empty()))
                .unwrap_or(m.user.name);
            cache_put(uid64, name.clone());
            return name;
        }
    }

    // 2) global (no guild nick): cache user -> HTTP user
    if let Some(u) = ctx.cache.user(UserId::new(uid64)) {
        let u = u.clone();
        let name = u.global_name.clone().filter(|s| !s.is_empty()).unwrap_or(u.name.clone());
        cache_put(uid64, name.clone());
        return name;
    }
    match UserId::new(uid64).to_user(&ctx.http).await {
        Ok(u) => {
            let name = u.global_name.unwrap_or(u.name);
            cache_put(uid64, name.clone());
            name
        }
        Err(_) => format!("user {}", uid64),
    }
}
pub fn mention_user(id: i64) -> String {
    UserId::new(id as u64).mention().to_string()
}

pub fn user_name(ctx: &Context, user_id: i64) -> String {
    let uid = UserId::new(user_id as u64);

    // 1) Prefer a nickname from any cached guild we have
    // (Serenity 0.12 cache exposes the set of guild IDs)
    let guild_ids: Vec<GuildId> = ctx.cache.guilds().iter().copied().collect();
    for gid in guild_ids {
        if let Some(g) = ctx.cache.guild(gid) {
            if let Some(m) = g.members.get(&uid) {
                // Nickname first (server-specific)
                if let Some(nick) = m.nick.as_deref() {
                    if !nick.is_empty() {
                        return nick.to_string();
                    }
                }
                // Then global display name (Discord-wide)
                if let Some(gn) = m.user.global_name.as_deref() {
                    if !gn.is_empty() {
                        return gn.to_string();
                    }
                }
                // Finally username
                return m.user.name.clone();
            }
        }
    }

    // 2) If we didn’t find a member anywhere, try the cached user
    if let Some(u) = ctx.cache.user(uid) {
        // `u` is an Arc — clone the fields we need
        let u = u.clone();
        return u.global_name.clone().filter(|s| !s.is_empty()).unwrap_or(u.name.clone());
    }

    // 3) Last-resort fallback (no cache)
    format!("user {}", uid.get())
}
pub fn user_name_in_guild(ctx: &Context, guild_id: u64, user_id: i64) -> String {
    let uid = UserId::new(user_id as u64);
    if let Some(g) = ctx.cache.guild(GuildId::new(guild_id)) {
        if let Some(m) = g.members.get(&uid) {
            if let Some(nick) = m.nick.as_deref() {
                if !nick.is_empty() {
                    return nick.to_string();
                }
            }
            if let Some(gn) = m.user.global_name.as_deref() {
                if !gn.is_empty() {
                    return gn.to_string();
                }
            }
            return m.user.name.clone();
        }
    }
    // Fallback to global cache resolution
    user_name(ctx, user_id)
}
pub async fn _user_name_in_guild_async(ctx: &Context, guild_id: u64, user_id: i64) -> String {
    let uid = UserId::new(user_id as u64);
    let gid = GuildId::new(guild_id);

    // Try cache-first
    let cached = user_name_in_guild(ctx, guild_id, user_id);
    if !cached.starts_with("user ") { // crude but effective "found" check
        return cached;
    }

    // Not cached — try fetching the member to get nickname
    if let Ok(member) = gid.member(&ctx.http, uid).await {
        if let Some(nick) = member.nick {
            if !nick.is_empty() {
                return nick;
            }
        }
        if let Some(gn) = member.user.global_name {
            if !gn.is_empty() {
                return gn;
            }
        }
        return member.user.name;
    }

    // Fall back to fetching the user (no guild nickname)
    match uid.to_user(&ctx.http).await {
        Ok(u) => u.global_name.unwrap_or(u.name),
        Err(_) => format!("user {}", uid.get()),
    }
}
/* custom_id formats used */
pub fn parse_component_id(s: &str) -> Option<(String, String, Uuid)> {
    let s = s.strip_prefix("r:").unwrap_or(s);
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        ["j","m",uuid] => uuid.parse().ok().map(|u| ("j".into(),"m".into(),u)),
        ["j","a",uuid] => uuid.parse().ok().map(|u| ("j".into(),"a".into(),u)),
        ["l",uuid]     => uuid.parse().ok().map(|u| ("l".into(),"".into(),u)),
        ["la",uuid]    => uuid.parse().ok().map(|u| ("la".into(),"".into(),u)),
        ["mg",uuid]    => uuid.parse().ok().map(|u| ("mg".into(),"".into(),u)),
        ["pc",uuid]    => uuid.parse().ok().map(|u| ("pc".into(),"".into(),u)),
        ["ps",uuid]    => uuid.parse().ok().map(|u| ("ps".into(),"".into(),u)),
        ["ok",uuid]    => uuid.parse().ok().map(|u| ("ok".into(),"".into(),u)),
        ["pr",uuid]    => uuid.parse().ok().map(|u| ("pr".into(),"".into(),u)),
        ["mr",uuid]    => uuid.parse().ok().map(|u| ("mr".into(),"".into(),u)),
        ["kk",uuid]    => uuid.parse().ok().map(|u| ("kk".into(),"".into(),u)),
        ["cx",uuid]    => uuid.parse().ok().map(|u| ("cx".into(),"".into(),u)),
        ["not",uuid]    => uuid.parse().ok().map(|u| ("not".into(),"".into(),u)),
        ["cho",uuid]    => uuid.parse().ok().map(|u| ("cho".into(),"".into(),u)),
        ["chp",uuid]    => uuid.parse().ok().map(|u| ("chp".into(),"".into(),u)),
        ["chc",uuid]    => uuid.parse().ok().map(|u| ("chc".into(),"".into(),u)),

        _ => None,
    }
}

/* Parse "HH:MM YYYY-MM-DD" in Europe/Warsaw -> UTC */
pub fn parse_raid_datetime(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 2 { return None; }
    let time = NaiveTime::parse_from_str(parts[0], "%H:%M").ok()?;
    let date = NaiveDate::parse_from_str(parts[1], "%Y-%m-%d").ok()?;
    let local = Warsaw.from_local_datetime(&NaiveDateTime::new(date, time)).single()?;
    Some(local.with_timezone(&Utc))
}

/* Weekday key string used to find category (e.g., "tuesday") */
pub fn weekday_key(dt_utc: chrono::DateTime<chrono::Utc>) -> &'static str {
    match dt_utc.with_timezone(&Warsaw).weekday() {
        chrono::Weekday::Mon => "monday",
        chrono::Weekday::Tue => "tuesday",
        chrono::Weekday::Wed => "wednesday",
        chrono::Weekday::Thu => "thursday",
        chrono::Weekday::Fri => "friday",
        chrono::Weekday::Sat => "saturday",
        chrono::Weekday::Sun => "sunday",
    }
}

// Returns <:name:id> for a guild emoji if found
pub fn emoji_tag(ctx: &Context, guild_id: u64, name: &str) -> Option<String> {
    let cache = ctx.cache.clone();
    let guild = cache.guild(serenity::all::GuildId::new(guild_id))?;
    let emoji = guild.emojis.iter().find(|(_, e)| e.name.eq_ignore_ascii_case(name)).map(|(_, e)| e);
    emoji.map(|e: &Emoji| format!("<:{}:{}>", e.name, e.id.get()))
}

/// Finds a duration in `text` and returns (cleaned_text, hours).
/// Recognizes: "2h", "2 h", "2hr", "2hours", "2 godz", "2godzina/2godziny/2godzin",
/// loose "g" / "gorziny" (common typo), and also minutes: "90m", "90 min", etc.
/// If nothing found -> (original text, 1.0).
pub fn extract_duration_hours(text: &str) -> (String, f64) {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?ix)
            \b
            (?P<num>\d+(?:[.,]\d+)?)
            \s*
            (?P<unit>
                h|hr|hrs|hour|hours|
                m|min|mins|minute|minutes|
                godz(?:ina|iny|in)?|g(?:odz)?|gorziny
            )
            \b
        "#
        )
            .unwrap()
    });

    if let Some(m) = RE.find(text) {
        // numeric value
        let caps = RE.captures(m.as_str()).unwrap();
        let num = caps["num"].replace(',', ".").parse::<f64>().unwrap_or(1.0);
        let unit = &caps["unit"].to_ascii_lowercase();

        // convert to hours
        let hours = match unit.as_str() {
            "m" | "min" | "mins" | "minute" | "minutes" => (num / 60.0).max(0.0),
            _ => num.max(0.0),
        };

        // remove the matched fragment from description
        let mut cleaned = String::new();
        cleaned.push_str(&text[..m.start()]);
        cleaned.push_str(&text[m.end()..]);

        // tidy double spaces and stray punctuation near removed token
        let cleaned = cleaned
            .replace("  ", " ")
            .replace(" ,", ",")
            .replace("( )", "")
            .trim()
            .to_string();

        let hours = if hours > 0.0 { hours } else { 1.0 };
        return (cleaned, hours);
    }

    (text.to_string(), 1.0)
}

/// Format hours nicely: "2h" or "1.5h"
pub fn fmt_hours(h: f64) -> String {
    if (h.fract()).abs() < 0.01 {
        format!("{}h", h.round() as i64)
    } else {
        // 1 decimal place looks neat
        format!("{:.1}h", h)
    }
}




pub async fn dm_user(http: &Http, user_id: u64, content: String) {
    let uid = UserId::new(user_id);
    if let Ok(dm) = uid.create_dm_channel(http).await {
        let _ = dm.say(http, content).await;
    }
}



/// Manual “notify now” (owner/organiser-only), same message as the scheduled one
pub async fn notify_raid_now(ctx: &Context, raid_id: uuid::Uuid) -> anyhow::Result<()> {
    let pool = pool_from_ctx(ctx).await?;
    let raid = crate::db::repo::get_raid(&pool, raid_id).await?;
    let parts = crate::db::repo::list_participants(&pool, raid_id).await?;
    let when_local = raid.scheduled_for.with_timezone(&Warsaw).format("%Y-%m-%d %H:%M %Z");
    let chan_mention = format!("<#{}>", raid.channel_id as u64);
    for p in parts {
        let status = if p.is_main { "MAIN" } else { "RESERVE" };
        let msg = format!(
            "📣 Notification: **{}** at **{}**.\nChannel: {}\nYour status: **{}**",
            raid.raid_name, when_local, chan_mention, status
        );
        dm_user(&ctx.http, p.user_id as u64, msg).await;
    }
    Ok(())
}
