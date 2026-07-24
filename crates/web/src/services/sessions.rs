//! Session lifecycle: idempotent create-or-update keyed on the client's own identifier.

use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde::Deserialize;

use entity::{sessions, users};

/// The session kinds a client may declare. Validated here rather than in the schema so
/// adding one is a code change, not a migration.
pub const KINDS: [&str; 4] = ["monitor", "net", "contest", "pota"];

/// The default when a client doesn't say — a receive-only monitoring run.
pub const DEFAULT_KIND: &str = "monitor";

pub fn is_valid_kind(kind: &str) -> bool {
    KINDS.contains(&kind)
}

/// What a client can assert about a session.
///
/// Every field except `client_key` is optional, and `None` means "don't touch this",
/// not "set this to null". That distinction matters: the desktop app embeds a lean
/// session blob in every observation batch, and those repeated writes must not blank
/// out a label the operator set in the browser.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionInput {
    pub client_key: String,
    pub kind: Option<String>,
    pub label: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub band: Option<String>,
    pub mode: Option<String>,
    pub frequency_mhz: Option<f64>,
    pub operator_callsign: Option<String>,
    pub grid: Option<String>,
    pub notes: Option<String>,
    pub source: Option<String>,
}

/// Find a session by its client-supplied key, scoped to the owner.
pub async fn find_by_client_key<C: ConnectionTrait>(
    db: &C,
    user_id: i64,
    client_key: &str,
) -> Result<Option<sessions::Model>, sea_orm::DbErr> {
    sessions::Entity::find()
        .filter(sessions::Column::UserId.eq(user_id))
        .filter(sessions::Column::ClientKey.eq(client_key))
        .one(db)
        .await
}

/// The result of an upsert, so a handler can pick 201 vs 200 without a second query.
pub struct Upserted {
    pub session: sessions::Model,
    pub created: bool,
}

/// Create the session if its `client_key` is new, otherwise update the fields the input
/// actually carries.
///
/// Unknown keys are *created* rather than rejected, which is what makes the client's
/// upload queue order-independent: a "session ended" message replayed from a spool
/// before its "session opened" partner still lands correctly.
pub async fn upsert_by_client_key<C: ConnectionTrait>(
    db: &C,
    user_id: i64,
    input: &SessionInput,
) -> Result<Upserted, sea_orm::DbErr> {
    let now = Utc::now();

    if let Some(existing) = find_by_client_key(db, user_id, &input.client_key).await? {
        let mut active = existing.into_active_model();

        // Only `Some(..)` overwrites — see the note on `SessionInput`.
        if let Some(v) = &input.kind {
            active.kind = Set(v.clone());
        }
        if let Some(v) = &input.label {
            active.label = Set(Some(v.clone()));
        }
        if let Some(v) = input.started_at {
            active.started_at = Set(v);
        }
        if let Some(v) = input.ended_at {
            active.ended_at = Set(Some(v));
        }
        if let Some(v) = &input.band {
            active.band = Set(Some(v.clone()));
        }
        if let Some(v) = &input.mode {
            active.mode = Set(Some(v.clone()));
        }
        if let Some(v) = input.frequency_mhz {
            active.frequency_mhz = Set(Some(v));
        }
        if let Some(v) = &input.operator_callsign {
            active.operator_callsign = Set(Some(v.clone()));
        }
        if let Some(v) = &input.grid {
            active.grid = Set(Some(v.clone()));
        }
        if let Some(v) = &input.notes {
            active.notes = Set(Some(v.clone()));
        }
        if let Some(v) = &input.source {
            active.source = Set(Some(v.clone()));
        }
        active.updated_at = Set(now);

        let session = active.update(db).await?;
        return Ok(Upserted {
            session,
            created: false,
        });
    }

    let session = sessions::ActiveModel {
        user_id: Set(user_id),
        client_key: Set(input.client_key.clone()),
        kind: Set(input
            .kind
            .clone()
            .unwrap_or_else(|| DEFAULT_KIND.to_string())),
        label: Set(input.label.clone()),
        // A session created from a replayed "close" has no start time of its own; now
        // is the only defensible guess, and the client will correct it when the real
        // open message arrives.
        started_at: Set(input.started_at.unwrap_or(now)),
        ended_at: Set(input.ended_at),
        band: Set(input.band.clone()),
        mode: Set(input.mode.clone()),
        frequency_mhz: Set(input.frequency_mhz),
        operator_callsign: Set(input.operator_callsign.clone()),
        grid: Set(input.grid.clone()),
        source: Set(input.source.clone()),
        notes: Set(input.notes.clone()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(db)
    .await?;

    Ok(Upserted {
        session,
        created: true,
    })
}

/// The callsign and grid actually in force for a session: the session's own values when
/// set, otherwise the operator's defaults from `users`.
pub fn effective_operator(
    session: &sessions::Model,
    user: &users::Model,
) -> (Option<String>, Option<String>) {
    let callsign = session
        .operator_callsign
        .clone()
        .or_else(|| user.callsign.clone());
    let grid = session.grid.clone().or_else(|| user.grid.clone());
    (callsign, grid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{seed_user, test_db};

    fn input(client_key: &str) -> SessionInput {
        SessionInput {
            client_key: client_key.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn kind_validation_accepts_only_known_kinds() {
        assert!(is_valid_kind("monitor"));
        assert!(is_valid_kind("contest"));
        assert!(!is_valid_kind("Monitor"));
        assert!(!is_valid_kind("rag-chew"));
        assert!(!is_valid_kind(""));
    }

    #[actix_web::test]
    async fn upsert_creates_then_updates_the_same_row() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;

        let first = upsert_by_client_key(&db, user.id, &input("abc"))
            .await
            .unwrap();
        assert!(first.created);
        assert_eq!(first.session.kind, "monitor");

        let mut second_input = input("abc");
        second_input.kind = Some("contest".to_string());
        let second = upsert_by_client_key(&db, user.id, &second_input)
            .await
            .unwrap();
        assert!(!second.created);
        assert_eq!(second.session.id, first.session.id);
        assert_eq!(second.session.kind, "contest");
    }

    #[actix_web::test]
    async fn upsert_leaves_unset_fields_alone() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;

        let mut labelled = input("abc");
        labelled.label = Some("Tuesday ARES net".to_string());
        labelled.band = Some("2m".to_string());
        upsert_by_client_key(&db, user.id, &labelled).await.unwrap();

        // A lean follow-up blob (what a batch upload carries) must not blank the label.
        let lean = input("abc");
        let after = upsert_by_client_key(&db, user.id, &lean).await.unwrap();
        assert_eq!(after.session.label.as_deref(), Some("Tuesday ARES net"));
        assert_eq!(after.session.band.as_deref(), Some("2m"));
    }

    #[actix_web::test]
    async fn upsert_creates_on_an_unknown_key_so_a_replayed_close_still_lands() {
        let db = test_db().await;
        let user = seed_user(&db, "op@example.com").await;

        let ended = Utc::now();
        let mut close_only = input("never-opened");
        close_only.ended_at = Some(ended);

        let result = upsert_by_client_key(&db, user.id, &close_only)
            .await
            .unwrap();
        assert!(result.created);
        assert!(result.session.ended_at.is_some());
    }

    #[actix_web::test]
    async fn client_keys_are_scoped_per_user() {
        let db = test_db().await;
        let a = seed_user(&db, "a@example.com").await;
        let b = seed_user(&db, "b@example.com").await;

        let one = upsert_by_client_key(&db, a.id, &input("shared"))
            .await
            .unwrap();
        let two = upsert_by_client_key(&db, b.id, &input("shared"))
            .await
            .unwrap();

        assert!(one.created && two.created);
        assert_ne!(one.session.id, two.session.id);
    }

    #[actix_web::test]
    async fn effective_operator_prefers_the_session_then_the_user() {
        let db = test_db().await;
        let mut user = seed_user(&db, "op@example.com").await;
        user.callsign = Some("W4USR".to_string());
        user.grid = Some("FM07".to_string());

        let mut session = upsert_by_client_key(&db, user.id, &input("k"))
            .await
            .unwrap()
            .session;

        // Nothing on the session: fall back to the user's own station.
        assert_eq!(
            effective_operator(&session, &user),
            (Some("W4USR".into()), Some("FM07".into()))
        );

        // A club call for this run overrides the default.
        session.operator_callsign = Some("W4CLUB".to_string());
        assert_eq!(
            effective_operator(&session, &user),
            (Some("W4CLUB".into()), Some("FM07".into()))
        );
    }
}
