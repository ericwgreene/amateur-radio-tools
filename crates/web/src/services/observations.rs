//! Batch ingest of heard transmissions.
//!
//! The desktop monitor produces observations continuously and uploads them in batches,
//! retrying whatever it could not deliver. Two properties follow from that, and both
//! are load-bearing:
//!
//! * **Replays are free.** Every item carries a client-generated `client_key`; a key
//!   already stored is counted as a duplicate, not an error.
//! * **A bad item never fails the batch.** If one unparseable callsign returned 400 for
//!   the whole request, the client would retry forever, its spool would grow without
//!   bound, and no observation would ever land again. Rejections are reported per item
//!   so the client can drop them and move on.

use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect, Set,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::services::sessions::{self, SessionInput};
use crate::services::stations::{self as station_service, HeardInput};
use crate::tools::callsign;
use entity::{contacts, observations, stations};

/// The most observations one request may carry.
///
/// The server mounts `web::JsonConfig::default()`, a 2 MB body limit, so this is really
/// a restatement of that in units the client can reason about: 200 rows with transcripts
/// stays comfortably inside it, and is roughly three hours of busy net traffic.
pub const MAX_BATCH: usize = 200;

/// One heard transmission as the client reports it.
#[derive(Debug, Clone, Deserialize)]
pub struct ObservationInput {
    pub client_key: String,
    pub callsign: String,
    pub heard_at: DateTime<Utc>,
    pub duration_secs: Option<f64>,
    pub band: Option<String>,
    pub mode: Option<String>,
    pub frequency_mhz: Option<f64>,
    /// Licensee details the client resolved. Folded into the station rollup rather than
    /// stored per observation — the high-volume table shouldn't carry duplicates.
    pub name: Option<String>,
    pub qth: Option<String>,
    pub grid: Option<String>,
    /// Only present when the operator opted in to uploading transcript text.
    pub transcript: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Rejected {
    pub client_key: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct IngestOutcome {
    pub session_id: i64,
    pub accepted: usize,
    pub duplicates: usize,
    pub rejected: Vec<Rejected>,
    pub stations_touched: usize,
}

/// Ingest a batch against a session, creating or updating the session as a side effect.
///
/// The session is passed in full rather than by id so the client never needs to learn a
/// server-assigned identifier — it can start recording offline and let the first
/// successful upload establish the session.
///
/// Takes a concrete connection rather than a generic one because the whole batch runs in
/// a transaction, and the rollup writes are only correct if they commit or roll back
/// together with the inserts.
pub async fn ingest_batch(
    db: &DatabaseConnection,
    user_id: i64,
    session: &SessionInput,
    items: Vec<ObservationInput>,
) -> Result<IngestOutcome, sea_orm::DbErr> {
    let tx = db.begin().await?;

    let session_id = sessions::upsert_by_client_key(&tx, user_id, session)
        .await?
        .session
        .id;

    // 1. Validate. Anything malformed is rejected here and never retried by the client.
    let mut rejected = Vec::new();
    let mut candidates = Vec::new();
    for item in items {
        // A blank key can't do its job. Worse, two of them in one batch would
        // collide on the unique index and turn the whole request into a 500 —
        // which is precisely the wedge the per-item rejection exists to avoid,
        // so it has to be caught here rather than by the database.
        if item.client_key.trim().is_empty() {
            rejected.push(Rejected {
                client_key: item.client_key,
                reason: "client_key is required and must not be blank".to_string(),
            });
            continue;
        }
        let normalized = callsign::normalize(&item.callsign);
        if !callsign::is_valid(&normalized) {
            rejected.push(Rejected {
                client_key: item.client_key,
                reason: format!("'{}' is not a valid callsign", item.callsign),
            });
            continue;
        }
        candidates.push((normalized, item));
    }

    // 2. Find which keys we already hold, in one query rather than one per item.
    let keys: Vec<String> = candidates
        .iter()
        .map(|(_, item)| item.client_key.clone())
        .collect();
    let seen: std::collections::HashSet<String> = if keys.is_empty() {
        std::collections::HashSet::new()
    } else {
        observations::Entity::find()
            .select_only()
            .column(observations::Column::ClientKey)
            .filter(observations::Column::UserId.eq(user_id))
            .filter(observations::Column::ClientKey.is_in(keys))
            .into_tuple::<String>()
            .all(&tx)
            .await?
            .into_iter()
            .collect()
    };

    // A batch can also repeat a key within itself; treat the second one as a duplicate
    // rather than letting the unique index turn it into a 500.
    let mut in_batch = std::collections::HashSet::new();
    let mut duplicates = 0usize;
    let mut fresh = Vec::new();
    for (normalized, item) in candidates {
        if seen.contains(&item.client_key) || !in_batch.insert(item.client_key.clone()) {
            duplicates += 1;
            continue;
        }
        fresh.push((normalized, item));
    }

    // 3. Insert the survivors, inheriting the session's radio metadata where the item
    //    doesn't override it.
    let now = Utc::now();
    let mut rows = Vec::with_capacity(fresh.len());
    for (normalized, item) in &fresh {
        let country = callsign::lookup(normalized).ok().map(|info| info.country);
        rows.push(observations::ActiveModel {
            user_id: Set(user_id),
            session_id: Set(session_id),
            client_key: Set(item.client_key.clone()),
            callsign: Set(normalized.clone()),
            heard_at: Set(item.heard_at),
            duration_secs: Set(item.duration_secs),
            band: Set(item.band.clone().or_else(|| session.band.clone())),
            mode: Set(item.mode.clone().or_else(|| session.mode.clone())),
            frequency_mhz: Set(item.frequency_mhz.or(session.frequency_mhz)),
            country: Set(country),
            transcript: Set(item.transcript.clone()),
            source: Set(item.source.clone().or_else(|| session.source.clone())),
            promoted_contact_id: Set(None),
            created_at: Set(now),
            ..Default::default()
        });
    }
    let accepted = rows.len();
    if !rows.is_empty() {
        observations::Entity::insert_many(rows).exec(&tx).await?;
    }

    // 4. Fold into the station rollup. One call per hearing keeps `times_heard` honest.
    let mut touched = std::collections::HashSet::new();
    for (normalized, item) in &fresh {
        station_service::record_heard(
            &tx,
            user_id,
            &HeardInput {
                callsign: normalized.clone(),
                heard_at: item.heard_at,
                name: item.name.clone(),
                qth: item.qth.clone(),
                grid: item.grid.clone(),
                country: callsign::lookup(normalized).ok().map(|i| i.country),
            },
        )
        .await?;
        touched.insert(normalized.clone());
    }

    tx.commit().await?;

    Ok(IngestOutcome {
        session_id,
        accepted,
        duplicates,
        rejected,
        stations_touched: touched.len(),
    })
}

/// QSO details supplied when promoting a hearing. All optional: what isn't given
/// is carried over from the observation and the station rollup.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct PromoteDetails {
    pub rst_sent: Option<String>,
    pub rst_received: Option<String>,
    pub worked_at: Option<DateTime<Utc>>,
    pub band: Option<String>,
    pub mode: Option<String>,
    pub frequency_mhz: Option<f64>,
    pub notes: Option<String>,
}

/// The outcome of a promote. `created` is false when the observation had already
/// been promoted, so a caller can answer 201 vs 200.
pub struct Promotion {
    pub contact: contacts::Model,
    pub created: bool,
}

/// Turn a heard transmission into a logbook contact.
///
/// Hearing a station and working it are different events, which is why they live
/// in different tables; this is the bridge between them.
///
/// Promoting the same observation twice must converge on one contact rather than
/// logging the QSO again — a client retrying a request whose response it never
/// saw, or an operator double-clicking the button, should be harmless. Two things
/// enforce that, and neither is sufficient alone:
///
/// * the insert and the back-reference share a transaction, so a failure between
///   them can't leave a contact that no observation points at (which the next
///   attempt would then duplicate); and
/// * the back-reference is written with a `promoted_contact_id IS NULL` guard, so
///   if a concurrent promote won the race we roll our own insert back and return
///   the contact that actually landed.
///
/// Returns `None` when the observation doesn't exist or isn't the caller's.
pub async fn promote(
    db: &DatabaseConnection,
    user_id: i64,
    observation_id: i64,
    details: PromoteDetails,
) -> Result<Option<Promotion>, sea_orm::DbErr> {
    let Some(observation) = observations::Entity::find_by_id(observation_id)
        .filter(observations::Column::UserId.eq(user_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };

    // Already promoted: hand back the contact we made last time.
    if let Some(existing) = promoted_contact(db, user_id, &observation).await? {
        return Ok(Some(Promotion {
            contact: existing,
            created: false,
        }));
    }

    // Carry over whatever the station rollup already knows, so a promoted contact
    // isn't emptier than the roster row it came from.
    let station = stations::Entity::find()
        .filter(stations::Column::UserId.eq(user_id))
        .filter(stations::Column::Callsign.eq(&observation.callsign))
        .one(db)
        .await?;

    let tx = db.begin().await?;
    let now = Utc::now();
    let contact = contacts::ActiveModel {
        user_id: Set(user_id),
        callsign: Set(observation.callsign.clone()),
        worked_at: Set(details.worked_at.unwrap_or(observation.heard_at)),
        band: Set(details.band.or_else(|| observation.band.clone())),
        mode: Set(details.mode.or_else(|| observation.mode.clone())),
        frequency_mhz: Set(details.frequency_mhz.or(observation.frequency_mhz)),
        rst_sent: Set(details.rst_sent),
        rst_received: Set(details.rst_received),
        grid: Set(station.as_ref().and_then(|s| s.grid.clone())),
        name: Set(station.as_ref().and_then(|s| s.name.clone())),
        qth: Set(station.as_ref().and_then(|s| s.qth.clone())),
        country: Set(observation.country.clone()),
        notes: Set(details.notes),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&tx)
    .await?;

    // The guard: only claim the observation if nobody else already has.
    let claimed = observations::Entity::update_many()
        .col_expr(
            observations::Column::PromotedContactId,
            Expr::value(contact.id),
        )
        .filter(observations::Column::Id.eq(observation_id))
        .filter(observations::Column::UserId.eq(user_id))
        .filter(observations::Column::PromotedContactId.is_null())
        .exec(&tx)
        .await?;

    if claimed.rows_affected == 0 {
        // A concurrent promote beat us. Roll back — which also discards the
        // contact we just inserted — and return whichever one won.
        tx.rollback().await?;
        let winner = promoted_contact(db, user_id, &observation).await?;
        return Ok(winner.map(|contact| Promotion {
            contact,
            created: false,
        }));
    }

    tx.commit().await?;
    Ok(Some(Promotion {
        contact,
        created: true,
    }))
}

/// The contact an observation was already promoted into, if any.
async fn promoted_contact<C: sea_orm::ConnectionTrait>(
    db: &C,
    user_id: i64,
    observation: &observations::Model,
) -> Result<Option<contacts::Model>, sea_orm::DbErr> {
    // Re-read rather than trusting the copy we hold: a racing promote may have
    // set this since.
    let Some(fresh) = observations::Entity::find_by_id(observation.id)
        .filter(observations::Column::UserId.eq(user_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let Some(contact_id) = fresh.promoted_contact_id else {
        return Ok(None);
    };
    contacts::Entity::find_by_id(contact_id)
        .filter(contacts::Column::UserId.eq(user_id))
        .one(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{seed_user, test_db};
    use chrono::Duration;

    fn session(key: &str) -> SessionInput {
        SessionInput {
            client_key: key.to_string(),
            band: Some("2m".to_string()),
            mode: Some("FM".to_string()),
            frequency_mhz: Some(146.88),
            ..Default::default()
        }
    }

    fn obs(client_key: &str, callsign: &str, at: DateTime<Utc>) -> ObservationInput {
        ObservationInput {
            client_key: client_key.to_string(),
            callsign: callsign.to_string(),
            heard_at: at,
            duration_secs: Some(4.5),
            band: None,
            mode: None,
            frequency_mhz: None,
            name: None,
            qth: None,
            grid: None,
            transcript: None,
            source: None,
        }
    }

    #[actix_web::test]
    async fn ingest_accepts_and_rolls_up() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        let out = ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![obs("k1", "kr4nrc", t), obs("k2", "W4ABC", t)],
        )
        .await
        .unwrap();

        assert_eq!(out.accepted, 2);
        assert_eq!(out.duplicates, 0);
        assert_eq!(out.stations_touched, 2);
        assert!(out.rejected.is_empty());

        // Callsigns are normalized on the way in.
        let stored = observations::Entity::find().all(&db).await.unwrap();
        let mut calls: Vec<_> = stored.iter().map(|o| o.callsign.clone()).collect();
        calls.sort();
        assert_eq!(calls, vec!["KR4NRC", "W4ABC"]);
    }

    #[actix_web::test]
    async fn observations_inherit_session_radio_metadata() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;

        ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![obs("k1", "KR4NRC", Utc::now())],
        )
        .await
        .unwrap();

        let row = observations::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.band.as_deref(), Some("2m"));
        assert_eq!(row.mode.as_deref(), Some("FM"));
        assert_eq!(row.frequency_mhz, Some(146.88));
    }

    #[actix_web::test]
    async fn replaying_a_batch_inserts_nothing_the_second_time() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();
        let batch = || vec![obs("k1", "KR4NRC", t), obs("k2", "W4ABC", t)];

        ingest_batch(&db, user.id, &session("s1"), batch())
            .await
            .unwrap();
        let again = ingest_batch(&db, user.id, &session("s1"), batch())
            .await
            .unwrap();

        assert_eq!(again.accepted, 0);
        assert_eq!(again.duplicates, 2);
        assert_eq!(
            observations::Entity::find().all(&db).await.unwrap().len(),
            2
        );

        // Crucially, the rollup didn't double-count either.
        let station = stations::Entity::find()
            .filter(stations::Column::Callsign.eq("KR4NRC"))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(station.times_heard, 1);
    }

    #[actix_web::test]
    async fn a_key_repeated_within_one_batch_is_a_duplicate_not_a_crash() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        let out = ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![obs("same", "KR4NRC", t), obs("same", "KR4NRC", t)],
        )
        .await
        .unwrap();

        assert_eq!(out.accepted, 1);
        assert_eq!(out.duplicates, 1);
    }

    /// The property that keeps a client's retry loop from wedging forever.
    #[actix_web::test]
    async fn a_bad_callsign_is_rejected_without_failing_the_batch() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        let out = ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![
                obs("k1", "KR4NRC", t),
                obs("k2", "!!", t),
                obs("k3", "W4ABC", t),
            ],
        )
        .await
        .unwrap();

        assert_eq!(out.accepted, 2);
        assert_eq!(out.rejected.len(), 1);
        assert_eq!(out.rejected[0].client_key, "k2");
    }

    /// A blank key can't identify anything, and two of them in one batch would
    /// collide on the unique index — turning the whole request into a 500 and
    /// wedging the client's retry loop forever.
    #[actix_web::test]
    async fn a_blank_client_key_is_rejected_rather_than_reaching_the_index() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        let out = ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![
                obs("", "KR4NRC", t),
                obs("   ", "W4ABC", t),
                obs("good", "K4CQ", t),
            ],
        )
        .await
        .unwrap();

        assert_eq!(out.accepted, 1, "the well-formed row still lands");
        assert_eq!(out.rejected.len(), 2);
        assert!(out.rejected.iter().all(|r| r.reason.contains("client_key")));
    }

    #[actix_web::test]
    async fn promote_creates_a_contact_once_and_only_once() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![obs("k1", "KR4NRC", Utc::now())],
        )
        .await
        .unwrap();
        let observation = observations::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap();

        let first = promote(&db, user.id, observation.id, PromoteDetails::default())
            .await
            .unwrap()
            .expect("the observation exists");
        assert!(first.created);
        assert_eq!(first.contact.callsign, "KR4NRC");

        // Enrichment from the station rollup comes across.
        assert_eq!(first.contact.country.as_deref(), Some("United States"));

        let second = promote(&db, user.id, observation.id, PromoteDetails::default())
            .await
            .unwrap()
            .expect("still there");
        assert!(
            !second.created,
            "a replay reports that it didn't create one"
        );
        assert_eq!(second.contact.id, first.contact.id);

        let all = contacts::Entity::find().all(&db).await.unwrap();
        assert_eq!(all.len(), 1, "exactly one QSO, however many times we asked");
    }

    #[actix_web::test]
    async fn promote_honours_supplied_qso_details() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![obs("k1", "KR4NRC", Utc::now())],
        )
        .await
        .unwrap();
        let observation = observations::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap();

        let promotion = promote(
            &db,
            user.id,
            observation.id,
            PromoteDetails {
                rst_sent: Some("59".into()),
                rst_received: Some("57".into()),
                band: Some("20m".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(promotion.contact.rst_sent.as_deref(), Some("59"));
        assert_eq!(
            promotion.contact.band.as_deref(),
            Some("20m"),
            "an explicit band overrides the one inherited from the session"
        );
        // Unspecified fields still fall back to the observation.
        assert_eq!(promotion.contact.mode.as_deref(), Some("FM"));
    }

    #[actix_web::test]
    async fn promote_is_scoped_to_the_owner() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let other = seed_user(&db, "other@example.com").await;
        ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![obs("k1", "KR4NRC", Utc::now())],
        )
        .await
        .unwrap();
        let observation = observations::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .unwrap();

        assert!(
            promote(&db, other.id, observation.id, PromoteDetails::default())
                .await
                .unwrap()
                .is_none()
        );
        assert!(contacts::Entity::find().all(&db).await.unwrap().is_empty());
    }

    #[actix_web::test]
    async fn promoting_an_unknown_observation_is_none_not_an_error() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        assert!(
            promote(&db, user.id, 999_999, PromoteDetails::default())
                .await
                .unwrap()
                .is_none()
        );
    }

    #[actix_web::test]
    async fn transcript_is_stored_only_when_supplied() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        let mut with_text = obs("k2", "W4ABC", t);
        with_text.transcript = Some("this is net control".to_string());

        ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![obs("k1", "KR4NRC", t), with_text],
        )
        .await
        .unwrap();

        let rows = observations::Entity::find().all(&db).await.unwrap();
        let quiet = rows.iter().find(|o| o.callsign == "KR4NRC").unwrap();
        let noisy = rows.iter().find(|o| o.callsign == "W4ABC").unwrap();
        assert_eq!(quiet.transcript, None, "opt-in default is off");
        assert_eq!(noisy.transcript.as_deref(), Some("this is net control"));
    }

    #[actix_web::test]
    async fn the_session_is_created_by_the_first_batch() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;

        let out = ingest_batch(&db, user.id, &session("brand-new"), vec![])
            .await
            .unwrap();

        let stored = sessions::find_by_client_key(&db, user.id, "brand-new")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.id, out.session_id);
    }

    #[actix_web::test]
    async fn repeat_hearings_accumulate_across_batches() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;
        let t = Utc::now();

        ingest_batch(&db, user.id, &session("s1"), vec![obs("k1", "KR4NRC", t)])
            .await
            .unwrap();
        ingest_batch(
            &db,
            user.id,
            &session("s1"),
            vec![obs("k2", "KR4NRC", t + Duration::minutes(3))],
        )
        .await
        .unwrap();

        let station = stations::Entity::find().one(&db).await.unwrap().unwrap();
        assert_eq!(station.times_heard, 2);
        assert_eq!(station.last_heard_at, t + Duration::minutes(3));
    }
}
