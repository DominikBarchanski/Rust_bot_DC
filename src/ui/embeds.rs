use serenity::all::{Context, CreateEmbed,GuildId,UserId};
use crate::db::models::{Raid, RaidParticipant};
use crate::utils::emoji_tag;
use chrono::{DateTime, Utc};
use chrono_tz::Europe::Warsaw;

pub fn render_new_raid_embed(raid_name: &str, description: &str, scheduled_for: chrono::DateTime<chrono::Utc>, max_player:&i64) -> CreateEmbed {
    let when_local = scheduled_for.with_timezone(&Warsaw).format("%Y-%m-%d %H:%M %Z");
    CreateEmbed::new()
        .title(format!("Raid: {}", raid_name))
        .description(format!("**Date:** {}\n{}", when_local, render_empty_slots(*max_player )))
        .field("Description", description, false)
}

pub fn render_raid_embed(ctx: &Context, guild_id: u64, raid: &Raid, participants: &[RaidParticipant]) -> CreateEmbed {
    render_raid_embed_inner(Some((ctx, guild_id)), raid, participants)
}

pub fn render_raid_embed_plain(raid: &Raid, participants: &[RaidParticipant]) -> CreateEmbed {
    render_raid_embed_inner(None, raid, participants)
}

fn render_raid_embed_inner(ctx_guild: Option<(&Context, u64)>, raid: &Raid, participants: &[RaidParticipant]) -> CreateEmbed {
    let slots = raid.max_players.max(1) as usize;

    let mut mains: Vec<&RaidParticipant> = participants.iter().filter(|p| p.is_main).collect();
    mains.sort_by_key(|p| (p.is_alt, p.joined_at)); // non-alt mains first

    let mut reserves: Vec<&RaidParticipant> = participants.iter().filter(|p| !p.is_main).collect();
    reserves.sort_by_key(|p| (p.is_alt, p.joined_at)); // show non-alt reserves first


    let mut lines: Vec<String> = Vec::with_capacity(slots);
    for i in 0..slots {
        if let Some(p) = mains.get(i) {
            let label = decorate_joined_as(ctx_guild, &p.joined_as);
            let suffix_role = p.tag_suffix.as_str();
            let suffix = if p.is_alt { " (ALT)" } else { "" };
            lines.push(format!("{}. {} <@{}>{}{}", i + 1, label, p.user_id, suffix,suffix_role));
        } else {
            lines.push(format!("{}. [Empty]", i + 1));
        }
    }
    let when_local = raid.scheduled_for.with_timezone(&Warsaw).format("%Y-%m-%d %H:%M %Z");
    let mut e = CreateEmbed::new()
        .title(format!("Raid: {}", raid.raid_name))
        .description(format!("**Date:** {}\n{}", when_local, lines.join("\n")))
        .field("Description", &raid.description, false)
        .field(
            "Capacity",
            format!("{}/{} (alts allowed: {}, max_alts: {})",
                    mains.len().min(slots),
                    raid.max_players,
                    if raid.allow_alts { "yes" } else { "no" },
                    raid.max_alts
            ),
            true
        );

    if let Some(until) = raid.priority_until {
        e = e.field("Priority until", format!("{}", until), true);
    }

    // Reserves compact field (first 10)
    if !reserves.is_empty() {
        let mut rlines = Vec::new();
        for p in reserves.iter().take(10) {
            let label = decorate_joined_as(ctx_guild, &p.joined_as);
            let suffix = if p.is_alt { " (ALT)" } else { "" };
            let suffix_role = p.tag_suffix.as_str();
            rlines.push(format!("• {} <@{}>{}{}", label, p.user_id, suffix,suffix_role));
        }
        if reserves.len() > 10 {
            rlines.push(format!("... and {} more", reserves.len() - 10));
        }
        e = e.field("Reserves", rlines.join("\n"), false);
    }

    e
}

fn render_empty_slots(n: i64) -> String {
    (1..=n).map(|i| format!("{i}. [Empty]")).collect::<Vec<_>>().join("\n")
}

fn decorate_joined_as(ctx_guild: Option<(&Context, u64)>, text: &str) -> String {
    let mut out = String::new();
    let parts: Vec<&str> = text.split('/').map(|s| s.trim()).collect();
    if parts.len() == 2 {
        let class = parts[0];
        let sp = parts[1].to_ascii_uppercase().replace("SP", "SP");
        let abbr: String = match class.to_ascii_lowercase().as_str() {
            "warrior" | "msw" => "MSW".to_string(),
            "archer" | "arch" => "ARCH".to_string(),
            "swordsman" | "sword" => "SWORD".to_string(),
            "mage" | "mag" => "MAG".to_string(),
            other => other.to_ascii_uppercase(),
        };
        let emoji_name = format!("{}_{}", abbr, sp);
        if let Some((ctx, gid)) = ctx_guild {
            if let Some(tag) = emoji_tag(ctx, gid, &emoji_name) {
                out.push_str(&format!("{} ", tag));
            }
        }
    }
    out.push_str(text);
    out
}
