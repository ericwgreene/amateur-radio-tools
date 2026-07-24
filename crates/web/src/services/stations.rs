//! The unique-station rollup — the single write path for `stations`.
//!
//! Every observation folds into exactly one station row. The merge rules mirror what
//! the desktop monitor already does in memory when it dedupes its on-screen roster:
//! bump the count, widen the first/last window, and fill in licensee details only when
//! there is something to fill them with.

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QuerySelect, Set,
};

use entity::{contacts, stations};

/// One hearing, ready to be folded into the rollup.
pub struct HeardInput {
    /// Already normalized to uppercase by the caller.
    pub callsign: String,
    pub heard_at: DateTime<Utc>,
    pub name: Option<String>,
    pub qth: Option<String>,
    pub grid: Option<String>,
    pub country: Option<String>,
}

/// Prefer the incoming value, but never replace something we already know with nothing.
///
/// A callsign lookup that times out returns an empty result, and letting that overwrite
/// a name fetched successfully last week would make the roster get worse over time.
fn merge(existing: Option<String>, incoming: Option<String>) -> Option<String> {
    match incoming {
        Some(v) if !v.trim().is_empty() => Some(v),
        _ => existing,
    }
}

/// Fold one hearing into the `(user, callsign)` rollup, creating the row if this is the
/// first time this station has been heard.
///
/// Read-modify-write rather than an `ON CONFLICT` upsert: the counter increment and the
/// min/max widening both depend on the current value, and expressing that portably
/// across SQLite and PostgreSQL is more trouble than it is worth for a path that runs
/// once per heard transmission. The caller is expected to supply a transaction.
pub async fn record_heard<C: ConnectionTrait>(
    db: &C,
    user_id: i64,
    input: &HeardInput,
) -> Result<stations::Model, sea_orm::DbErr> {
    let now = Utc::now();

    let existing = stations::Entity::find()
        .filter(stations::Column::UserId.eq(user_id))
        .filter(stations::Column::Callsign.eq(&input.callsign))
        .one(db)
        .await?;

    let Some(row) = existing else {
        return stations::ActiveModel {
            user_id: Set(user_id),
            callsign: Set(input.callsign.clone()),
            first_heard_at: Set(input.heard_at),
            last_heard_at: Set(input.heard_at),
            times_heard: Set(1),
            name: Set(input.name.clone()),
            qth: Set(input.qth.clone()),
            grid: Set(input.grid.clone()),
            country: Set(input.country.clone()),
            notes: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(db)
        .await;
    };

    // Explicit min/max, not assignment. A batch that sat in an offline client's spool
    // for a week arrives carrying week-old timestamps; assigning them would drag
    // `last_heard_at` backwards and corrupt the ordering of the whole roster.
    let first_heard_at = row.first_heard_at.min(input.heard_at);
    let last_heard_at = row.last_heard_at.max(input.heard_at);
    let times_heard = row.times_heard + 1;

    let name = merge(row.name.clone(), input.name.clone());
    let qth = merge(row.qth.clone(), input.qth.clone());
    let grid = merge(row.grid.clone(), input.grid.clone());
    let country = merge(row.country.clone(), input.country.clone());

    let mut active = row.into_active_model();
    active.first_heard_at = Set(first_heard_at);
    active.last_heard_at = Set(last_heard_at);
    active.times_heard = Set(times_heard);
    active.name = Set(name);
    active.qth = Set(qth);
    active.grid = Set(grid);
    active.country = Set(country);
    active.updated_at = Set(now);
    active.update(db).await
}

/// How many logbook contacts exist for each of `callsigns`, for this user.
///
/// "Times worked" is derived rather than stored. Contacts can be created from the REST
/// API, the logbook form, or a seeding script, and a counter that only some of those
/// paths remembered to bump would quietly read zero for stations worked the ordinary
/// way. Counting on read cannot drift.
pub async fn worked_counts<C: ConnectionTrait>(
    db: &C,
    user_id: i64,
    callsigns: &[String],
) -> Result<std::collections::HashMap<String, i64>, sea_orm::DbErr> {
    if callsigns.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let rows: Vec<(String, i64)> = contacts::Entity::find()
        .select_only()
        .column(contacts::Column::Callsign)
        .column_as(contacts::Column::Id.count(), "count")
        .filter(contacts::Column::UserId.eq(user_id))
        .filter(contacts::Column::Callsign.is_in(callsigns.to_vec()))
        .group_by(contacts::Column::Callsign)
        .into_tuple()
        .all(db)
        .await?;

    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{seed_contact, seed_user, test_db};
    use chrono::Duration;

    fn heard(callsign: &str, at: DateTime<Utc>) -> HeardInput {
        HeardInput {
            callsign: callsign.to_string(),
            heard_at: at,
            name: None,
            qth: None,
            grid: None,
            country: None,
        }
    }

    #[test]
    fn merge_keeps_existing_when_incoming_is_blank() {
        assert_eq!(
            merge(Some("John".into()), None),
            Some("John".into()),
            "a failed lookup must not erase a known name"
        );
        assert_eq!(
            merge(Some("John".into()), Some("  ".into())),
            Some("John".into())
        );
        assert_eq!(
            merge(Some("John".into()), Some("Jane".into())),
            Some("Jane".into())
        );
        assert_eq!(merge(None, Some("Jane".into())), Some("Jane".into()));
        assert_eq!(merge(None, None), None);
    }

    #[actix_web::test]
    async fn first_hearing_initializes_the_row() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        let row = record_heard(&db, user.id, &heard("KR4NRC", t))
            .await
            .unwrap();

        assert_eq!(row.times_heard, 1);
        assert_eq!(row.first_heard_at, t);
        assert_eq!(row.last_heard_at, t);
    }

    #[actix_web::test]
    async fn later_hearing_advances_last_heard_and_counts() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        record_heard(&db, user.id, &heard("KR4NRC", t))
            .await
            .unwrap();
        let row = record_heard(&db, user.id, &heard("KR4NRC", t + Duration::hours(1)))
            .await
            .unwrap();

        assert_eq!(row.times_heard, 2);
        assert_eq!(row.first_heard_at, t);
        assert_eq!(row.last_heard_at, t + Duration::hours(1));
    }

    /// The late-spool-replay case: a client that was offline uploads old hearings.
    /// The count must rise, but the window must only ever widen.
    #[actix_web::test]
    async fn earlier_hearing_never_drags_last_heard_backwards() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        record_heard(&db, user.id, &heard("KR4NRC", t))
            .await
            .unwrap();
        let row = record_heard(&db, user.id, &heard("KR4NRC", t - Duration::days(7)))
            .await
            .unwrap();

        assert_eq!(row.times_heard, 2);
        assert_eq!(
            row.first_heard_at,
            t - Duration::days(7),
            "window widens back"
        );
        assert_eq!(row.last_heard_at, t, "but the latest hearing still stands");
    }

    #[actix_web::test]
    async fn licensee_details_fill_in_but_are_not_erased() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        let mut with_name = heard("KR4NRC", t);
        with_name.name = Some("John Smith".to_string());
        with_name.qth = Some("Lynchburg, VA".to_string());
        record_heard(&db, user.id, &with_name).await.unwrap();

        // A later hearing whose lookup failed carries nothing.
        let row = record_heard(&db, user.id, &heard("KR4NRC", t + Duration::minutes(5)))
            .await
            .unwrap();

        assert_eq!(row.name.as_deref(), Some("John Smith"));
        assert_eq!(row.qth.as_deref(), Some("Lynchburg, VA"));
    }

    #[actix_web::test]
    async fn stations_are_scoped_per_user() {
        let db = test_db().await;
        let a = seed_user(&db, "a@example.com").await;
        let b = seed_user(&db, "b@example.com").await;
        let t = Utc::now();

        record_heard(&db, a.id, &heard("KR4NRC", t)).await.unwrap();
        let theirs = record_heard(&db, b.id, &heard("KR4NRC", t)).await.unwrap();

        assert_eq!(theirs.times_heard, 1, "b's rollup is independent of a's");
    }

    #[actix_web::test]
    async fn worked_counts_come_from_the_logbook() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        seed_contact(&db, user.id, "KR4NRC").await;
        seed_contact(&db, user.id, "KR4NRC").await;
        seed_contact(&db, user.id, "W4ABC").await;

        let counts = worked_counts(
            &db,
            user.id,
            &[
                "KR4NRC".to_string(),
                "W4ABC".to_string(),
                "K4CQ".to_string(),
            ],
        )
        .await
        .unwrap();

        assert_eq!(counts.get("KR4NRC"), Some(&2));
        assert_eq!(counts.get("W4ABC"), Some(&1));
        assert_eq!(counts.get("K4CQ"), None, "never worked, so absent");
    }

    #[actix_web::test]
    async fn worked_counts_short_circuits_on_an_empty_list() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        assert!(worked_counts(&db, user.id, &[]).await.unwrap().is_empty());
    }
}
