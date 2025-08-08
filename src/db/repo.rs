use super::models::{Raid, RaidParticipant};
use chrono::{DateTime, Utc};
use sqlx::{types::Json, PgPool};
use uuid::Uuid;

/* RAIDS */

pub async fn create_raid_with_id(
    pool: &PgPool,
    id: Uuid,
    guild_id: i64,
    channel_id: i64,
    message_id: i64,
    scheduled_for: DateTime<Utc>,
    created_by: i64,
    owner_id: i64,
    description: String,
    priority_list: Vec<i64>,
    is_priority: bool,
    raid_name: String,
    max_players: i32,
    allow_alts: bool,
    max_alts: i32,
    priority_role_id: Option<i64>,
    priority_until: Option<DateTime<Utc>>,
) -> anyhow::Result<Raid> {
    let raid = sqlx::query_as!(
        Raid,
        r#"
        INSERT INTO raids (
            id, guild_id, channel_id, message_id, scheduled_for,
            created_by, owner_id, description, is_priority, is_active, priority_list,
            raid_name, max_players, allow_alts, max_alts, priority_role_id, priority_until
        )
        VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,TRUE,$10,$11,$12,$13,$14,$15,$16)
        RETURNING
            id, guild_id, channel_id, message_id, scheduled_for, created_by, owner_id,
            description, is_priority, is_active, priority_list as "priority_list: Json<Vec<i64>>",
            raid_name, max_players, allow_alts, max_alts, priority_role_id, priority_until
        "#,
        id, guild_id, channel_id, message_id, scheduled_for,
        created_by, owner_id, description, is_priority,
        Json(priority_list) as Json<Vec<i64>>,
        raid_name, max_players, allow_alts, max_alts, priority_role_id, priority_until
    )
        .fetch_one(pool)
        .await?;
    Ok(raid)
}

pub async fn get_raid(pool: &PgPool, raid_id: Uuid) -> anyhow::Result<Raid> {
    let raid = sqlx::query_as!(
        Raid,
        r#"
        SELECT id, guild_id, channel_id, message_id, scheduled_for, created_by, owner_id,
               description, is_priority, is_active, priority_list as "priority_list: Json<Vec<i64>>",
               raid_name, max_players, allow_alts, max_alts, priority_role_id, priority_until
        FROM raids
        WHERE id = $1
        "#,
        raid_id
    )
        .fetch_one(pool)
        .await?;
    Ok(raid)
}

/* PARTICIPANTS */

pub async fn list_participants(pool: &PgPool, raid_id: Uuid) -> anyhow::Result<Vec<RaidParticipant>> {
    let rows = sqlx::query_as!(
        RaidParticipant,
        r#"
        SELECT id, raid_id, user_id, is_main, joined_as, is_reserve, joined_at, is_alt, tag_suffix
        FROM raid_participants
        WHERE raid_id = $1
        ORDER BY is_main DESC, is_alt ASC, joined_at ASC
        "#,
        raid_id
    )
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn count_mains(pool: &PgPool, raid_id: Uuid) -> anyhow::Result<i64> {
    let rec = sqlx::query_scalar!(
        r#"SELECT COUNT(*)::BIGINT FROM raid_participants WHERE raid_id=$1 AND is_main=TRUE"#,
        raid_id
    ).fetch_one(pool).await?;
    Ok(rec.unwrap_or(0))
}

pub async fn count_alt_mains(pool: &PgPool, raid_id: Uuid) -> anyhow::Result<i64> {
    let rec = sqlx::query_scalar!(
        r#"SELECT COUNT(*)::BIGINT FROM raid_participants WHERE raid_id=$1 AND is_main=TRUE AND is_alt=TRUE"#,
        raid_id
    ).fetch_one(pool).await?;
    Ok(rec.unwrap_or(0))
}

pub async fn user_has_main(pool: &PgPool, raid_id: Uuid, user_id: i64) -> anyhow::Result<bool> {
    // SELECT EXISTS returns a plain bool for Postgres; no unwrap needed.
    let rec = sqlx::query_scalar!(
        r#"SELECT EXISTS(
            SELECT 1 FROM raid_participants
            WHERE raid_id=$1 AND user_id=$2 AND is_main=TRUE AND is_alt=FALSE
        )"#,
        raid_id,
        user_id
    )
        .fetch_one(pool)
        .await?;
    Ok(rec.expect("REASON"))
}


pub async fn alt_count_for_user(pool: &PgPool, raid_id: Uuid, user_id: i64) -> anyhow::Result<i64> {
    let rec = sqlx::query_scalar!(
        r#"SELECT COUNT(*)::BIGINT FROM raid_participants WHERE raid_id=$1 AND user_id=$2 AND is_alt=TRUE"#,
        raid_id, user_id
    ).fetch_one(pool).await?;
    Ok(rec.unwrap_or(0))
}

pub async fn insert_or_replace_main(
    pool: &PgPool,
    raid_id: Uuid,
    user_id: i64,
    joined_as: String,
    main_now: bool, // if false, goes to reserve (non-alt)
    tag_suffix: String,
) -> anyhow::Result<RaidParticipant> {
    // if user already has a main row, update it; otherwise insert
    let maybe = sqlx::query_as!(
        RaidParticipant,
        r#"
        UPDATE raid_participants
        SET joined_as = $1, is_main = $2, is_reserve = NOT $2, is_alt = FALSE, tag_suffix = $5
        WHERE raid_id = $3 AND user_id = $4 AND is_alt = FALSE
        RETURNING id, raid_id, user_id, is_main, joined_as, is_reserve, joined_at, is_alt, tag_suffix
        "#,
        joined_as, main_now, raid_id, user_id,tag_suffix
    ).fetch_optional(pool).await?;

    if let Some(row) = maybe {
        return Ok(row);
    }

    let id = Uuid::new_v4();
    let row = sqlx::query_as!(
        RaidParticipant,
        r#"
        INSERT INTO raid_participants (id, raid_id, user_id, is_main, joined_as, is_reserve, is_alt,tag_suffix)
        VALUES ($1,$2,$3,$4,$5,$6,FALSE,$7)
        RETURNING id, raid_id, user_id, is_main, joined_as, is_reserve, joined_at, is_alt,tag_suffix
        "#,
        id, raid_id, user_id, main_now, joined_as, !main_now,tag_suffix
    ).fetch_one(pool).await?;
    Ok(row)
}

pub async fn insert_alt(
    pool: &PgPool,
    raid_id: Uuid,
    user_id: i64,
    joined_as: String,
    main_now: bool, // if false → reserve alt
    tag_suffix: String,
) -> anyhow::Result<RaidParticipant> {
    let id = Uuid::new_v4();
    let row = sqlx::query_as!(
        RaidParticipant,
        r#"
        INSERT INTO raid_participants (id, raid_id, user_id, is_main, joined_as, is_reserve, is_alt,tag_suffix)
        VALUES ($1,$2,$3,$4,$5,$6,TRUE,$7)
        RETURNING id, raid_id, user_id, is_main, joined_as, is_reserve, joined_at, is_alt,tag_suffix
        "#,
        id, raid_id, user_id, main_now, joined_as, !main_now,tag_suffix
    ).fetch_one(pool).await?;
    Ok(row)
}

pub async fn remove_participant(pool: &PgPool, raid_id: Uuid, user_id: i64) -> anyhow::Result<u64> {
    let res = sqlx::query!(
        "DELETE FROM raid_participants WHERE raid_id = $1 AND user_id = $2",
        raid_id, user_id
    )
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

pub async fn remove_user_alts(pool: &PgPool, raid_id: Uuid, user_id: i64) -> anyhow::Result<u64> {
    let res = sqlx::query!(
        "DELETE FROM raid_participants WHERE raid_id = $1 AND user_id = $2 AND is_alt = TRUE",
        raid_id, user_id
    )
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/* PROMOTION: non-alt reserves first, then alt reserves within alt-cap */
pub async fn promote_reserves_with_alt_limits(
    pool: &PgPool,
    raid_id: Uuid,
    max_players: i32,
    max_alts: i32,
) -> anyhow::Result<()> {
    let mains = count_mains(pool, raid_id).await? as i32;
    let mut free = (max_players - mains).max(0);

    if free <= 0 {
        return Ok(());
    }

    // 1) Promote non-alt reserves
    let free_i64 = free as i64;
    let promoted_non_alt = sqlx::query_scalar!(
        r#"
        WITH c AS (
          SELECT id FROM raid_participants
          WHERE raid_id = $1 AND is_main = FALSE AND is_alt = FALSE
          ORDER BY joined_at ASC
          LIMIT $2
        )
        UPDATE raid_participants p
        SET is_main = TRUE, is_reserve = FALSE
        FROM c
        WHERE p.id = c.id
        RETURNING 1
        "#,
        raid_id, free_i64
    ).fetch_all(pool).await?.len() as i32;

    free -= promoted_non_alt;
    if free <= 0 {
        return Ok(());
    }

    // 2) Promote alt reserves within alt cap
    let current_alt_mains = count_alt_mains(pool, raid_id).await? as i32;
    let alt_left = (max_alts - current_alt_mains).max(0);
    if alt_left <= 0 {
        return Ok(());
    }

    let promote_alt = free.min(alt_left);
    let promote_alt_i64 = promote_alt as i64;
    let _promoted_alts = sqlx::query_scalar!(
        r#"
        WITH c AS (
          SELECT id FROM raid_participants
          WHERE raid_id = $1 AND is_main = FALSE AND is_alt = TRUE
          ORDER BY joined_at ASC
          LIMIT $2
        )
        UPDATE raid_participants p
        SET is_main = TRUE, is_reserve = FALSE
        FROM c
        WHERE p.id = c.id
        RETURNING 1
        "#,
        raid_id, promote_alt_i64
    ).fetch_all(pool).await?.len() as i32;

    Ok(())
}
