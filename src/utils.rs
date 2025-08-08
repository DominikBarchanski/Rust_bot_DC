use uuid::Uuid;
use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Utc};
use chrono::TimeZone;
use chrono::Datelike;
use chrono_tz::Europe::Warsaw;
use serenity::all::{Context, Emoji};

pub fn from_user_id(id: serenity::all::UserId) -> i64 { id.get() as i64 }

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
        ["kk",uuid]    => uuid.parse().ok().map(|u| ("kk".into(),"".into(),u)),
        ["cx",uuid]    => uuid.parse().ok().map(|u| ("cx".into(),"".into(),u)),
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
