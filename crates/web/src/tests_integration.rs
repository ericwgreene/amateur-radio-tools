//! End-to-end integration tests plus focused unit tests for otherwise-untested modules.
//!
//! HTTP tests drive the real route table (`routes::configure`) through `actix_web::test`
//! against an in-memory SQLite database, exercising the request → extractor → handler → DB
//! → template path offline. Direct tests cover pure/public items (config parsing, error
//! mapping, session helpers).

use actix_web::http::header;
use actix_web::{ResponseError, test};

use crate::api::error::ApiError;
use crate::auth::session::SessionUser;
use crate::config::Config;
use crate::error::AppError;
use crate::test_support::{
    login, seed_contact, seed_observation, seed_session, seed_station, seed_token, seed_user,
    session_user, test_app, test_state,
};

/// Build a urlencoded form POST request builder (call `.to_request()` at the use site).
fn form(uri: &str, body: &'static str) -> test::TestRequest {
    test::TestRequest::post()
        .uri(uri)
        .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
        .set_payload(body)
}

// ---------------------------------------------------------------------------
// Public HTMX tools (no auth)
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn grid_tool_ok_and_error() {
    let state = test_state().await;
    let app = test_app!(state);

    let resp = test::call_service(
        &app,
        form("/tools/grid", "lat=41.714775&lon=-72.727260").to_request(),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let body = test::read_body(resp).await;
    assert!(String::from_utf8_lossy(&body).contains("FN31pr"));

    let resp = test::call_service(&app, form("/tools/grid", "lat=abc&lon=0").to_request()).await;
    let body = test::read_body(resp).await;
    assert!(String::from_utf8_lossy(&body).contains("Latitude must be a number"));
}

#[actix_web::test]
async fn callsign_tool_ok_and_error() {
    let state = test_state().await;
    let app = test_app!(state);

    let resp =
        test::call_service(&app, form("/tools/callsign", "callsign=w1aw").to_request()).await;
    assert_eq!(resp.status(), 200);
    assert!(String::from_utf8_lossy(&test::read_body(resp).await).contains("United States"));

    let resp = test::call_service(
        &app,
        form("/tools/callsign", "callsign=%21%21").to_request(),
    )
    .await;
    assert!(String::from_utf8_lossy(&test::read_body(resp).await).contains("not a valid callsign"));
}

// ---------------------------------------------------------------------------
// Pages + session auth
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn home_renders_logged_out_and_in() {
    let state = test_state().await;
    let user = seed_user(&state.db, "home@example.com").await;
    let app = test_app!(state);

    let resp = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
    assert_eq!(resp.status(), 200);

    let cookie = login!(app, user.id, false);
    let req = test::TestRequest::get()
        .uri("/")
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn dashboard_requires_auth() {
    let state = test_state().await;
    let app = test_app!(state);

    // No session → 303 redirect to /login.
    let resp = test::call_service(
        &app,
        test::TestRequest::get().uri("/dashboard").to_request(),
    )
    .await;
    assert_eq!(resp.status(), 303);

    // HTMX request → 200 with HX-Redirect header instead.
    let req = test::TestRequest::get()
        .uri("/dashboard")
        .insert_header(("HX-Request", "true"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    assert!(resp.headers().contains_key("HX-Redirect"));
}

#[actix_web::test]
async fn dashboard_ok_when_authed() {
    let state = test_state().await;
    let user = seed_user(&state.db, "dash@example.com").await;
    let app = test_app!(state);
    let cookie = login!(app, user.id, false);
    let req = test::TestRequest::get()
        .uri("/dashboard")
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);
}

#[actix_web::test]
async fn admin_is_role_gated() {
    let state = test_state().await;
    let user = seed_user(&state.db, "admin@example.com").await;
    let app = test_app!(state);

    let cookie = login!(app, user.id, false);
    let req = test::TestRequest::get()
        .uri("/admin")
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 403);

    let cookie = login!(app, user.id, true);
    let req = test::TestRequest::get()
        .uri("/admin")
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);
}

// ---------------------------------------------------------------------------
// Logbook
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn logbook_add_list_delete() {
    let state = test_state().await;
    let user = seed_user(&state.db, "log@example.com").await;
    let app = test_app!(state);
    let cookie = login!(app, user.id, false);

    // Empty list renders.
    let req = test::TestRequest::get()
        .uri("/logbook")
        .cookie(cookie.clone())
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);

    // Add a contact (country auto-resolved from callsign).
    let add = test::TestRequest::post()
        .uri("/logbook")
        .cookie(cookie.clone())
        .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
        .set_payload("callsign=g3abc&band=20m&rst_sent=59&rst_received=57")
        .to_request();
    let resp = test::call_service(&app, add).await;
    assert_eq!(resp.status(), 200);
    let body = test::read_body(resp).await;
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("G3ABC"));
    assert!(body.contains("England"));

    // Empty callsign is a no-op that still returns the table body.
    let noop = test::TestRequest::post()
        .uri("/logbook")
        .cookie(cookie.clone())
        .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
        .set_payload("callsign=%20")
        .to_request();
    assert_eq!(test::call_service(&app, noop).await.status(), 200);

    // Delete a seeded contact owned by the user.
    let contact = seed_contact(&state.db, user.id, "K1ABC").await;
    let del = test::TestRequest::delete()
        .uri(&format!("/logbook/{}", contact.id))
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, del).await.status(), 200);
}

// ---------------------------------------------------------------------------
// Settings / API tokens
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn token_settings_create_and_revoke() {
    let state = test_state().await;
    let user = seed_user(&state.db, "tok@example.com").await;
    let app = test_app!(state);
    let cookie = login!(app, user.id, false);

    let req = test::TestRequest::get()
        .uri("/settings/tokens")
        .cookie(cookie.clone())
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 200);

    // Create shows the raw token once.
    let create = test::TestRequest::post()
        .uri("/settings/tokens")
        .cookie(cookie.clone())
        .insert_header((header::CONTENT_TYPE, "application/x-www-form-urlencoded"))
        .set_payload("name=WSJT-X")
        .to_request();
    let resp = test::call_service(&app, create).await;
    assert_eq!(resp.status(), 200);
    assert!(String::from_utf8_lossy(&test::read_body(resp).await).contains("art_"));

    // Revoke a seeded token owned by the user.
    let (_raw, token_id) = seed_token(&state.db, user.id).await;
    let revoke = test::TestRequest::post()
        .uri(&format!("/settings/tokens/{token_id}/revoke"))
        .cookie(cookie)
        .to_request();
    assert_eq!(test::call_service(&app, revoke).await.status(), 200);
}

// ---------------------------------------------------------------------------
// REST API (bearer token)
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn api_rejects_missing_and_bad_tokens() {
    let state = test_state().await;
    let app = test_app!(state);

    let req = test::TestRequest::post()
        .uri("/api/v1/contacts")
        .set_json(serde_json::json!({"callsign": "W1AW"}))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 401);

    let req = test::TestRequest::get()
        .uri("/api/v1/me")
        .insert_header(("Authorization", "Bearer art_bogus"))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 401);
}

#[actix_web::test]
async fn api_contacts_crud_with_token() {
    let state = test_state().await;
    let user = seed_user(&state.db, "api@example.com").await;
    let (raw, _id) = seed_token(&state.db, user.id).await;
    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    // Identify the token owner.
    let me = test::TestRequest::get()
        .uri("/api/v1/me")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    let resp = test::call_service(&app, me).await;
    assert_eq!(resp.status(), 200);
    assert!(String::from_utf8_lossy(&test::read_body(resp).await).contains("api@example.com"));

    // Create (callsign normalized + country resolved).
    let create = test::TestRequest::post()
        .uri("/api/v1/contacts")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(serde_json::json!({"callsign": "ve3xyz", "band": "40m"}))
        .to_request();
    let resp = test::call_service(&app, create).await;
    assert_eq!(resp.status(), 201);
    let created: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(created["callsign"], "VE3XYZ");
    assert_eq!(created["country"], "Canada");
    let id = created["id"].as_i64().unwrap();

    // Invalid callsign → 400.
    let bad = test::TestRequest::post()
        .uri("/api/v1/contacts")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(serde_json::json!({"callsign": "!!"}))
        .to_request();
    assert_eq!(test::call_service(&app, bad).await.status(), 400);

    // List.
    let list = test::TestRequest::get()
        .uri("/api/v1/contacts")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    assert_eq!(test::call_service(&app, list).await.status(), 200);

    // Get by id (found + not found).
    let get = test::TestRequest::get()
        .uri(&format!("/api/v1/contacts/{id}"))
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    assert_eq!(test::call_service(&app, get).await.status(), 200);

    let missing = test::TestRequest::get()
        .uri("/api/v1/contacts/999999")
        .insert_header(("Authorization", bearer))
        .to_request();
    assert_eq!(test::call_service(&app, missing).await.status(), 404);
}

// ---------------------------------------------------------------------------
// REST API — monitoring (sessions, observations, stations)
// ---------------------------------------------------------------------------

/// Body for a minimal observation batch.
fn batch(session_key: &str, observations: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "session": { "client_key": session_key, "kind": "monitor", "band": "2m" },
        "observations": observations,
    })
}

#[actix_web::test]
async fn api_sessions_create_is_idempotent_on_client_key() {
    let state = test_state().await;
    let user = seed_user(&state.db, "op@example.com").await;
    let (raw, _) = seed_token(&state.db, user.id).await;
    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    let body = serde_json::json!({"client_key": "run-1", "kind": "contest", "label": "Field Day"});

    let first = test::TestRequest::post()
        .uri("/api/v1/sessions")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(&body)
        .to_request();
    let resp = test::call_service(&app, first).await;
    assert_eq!(resp.status(), 201);
    let created: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(created["kind"], "contest");

    // A replay is a 200 on the same row, so a client that lost the first response can
    // tell it already landed.
    let again = test::TestRequest::post()
        .uri("/api/v1/sessions")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(&body)
        .to_request();
    let resp = test::call_service(&app, again).await;
    assert_eq!(resp.status(), 200);
    let replayed: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(replayed["id"], created["id"]);

    // An unknown kind is rejected rather than silently stored.
    let bad = test::TestRequest::post()
        .uri("/api/v1/sessions")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(serde_json::json!({"client_key": "run-2", "kind": "rag-chew"}))
        .to_request();
    assert_eq!(test::call_service(&app, bad).await.status(), 400);

    // So is a blank key.
    let blank = test::TestRequest::post()
        .uri("/api/v1/sessions")
        .insert_header(("Authorization", bearer))
        .set_json(serde_json::json!({"client_key": "  "}))
        .to_request();
    assert_eq!(test::call_service(&app, blank).await.status(), 400);
}

#[actix_web::test]
async fn api_sessions_patch_closes_and_filters_open() {
    let state = test_state().await;
    let user = seed_user(&state.db, "op@example.com").await;
    let (raw, _) = seed_token(&state.db, user.id).await;
    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    // PATCH on an unknown key creates it — that upsert is what makes a client's
    // upload queue order-independent.
    let close = test::TestRequest::patch()
        .uri("/api/v1/sessions/by-key/never-opened")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(serde_json::json!({"client_key": "ignored", "ended_at": "2026-07-24T23:00:00Z"}))
        .to_request();
    let resp = test::call_service(&app, close).await;
    assert_eq!(resp.status(), 200);
    let session: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(
        session["client_key"], "never-opened",
        "the path wins over the body"
    );
    assert!(!session["ended_at"].is_null());

    // A closed session is excluded from the open filter.
    let open = test::TestRequest::get()
        .uri("/api/v1/sessions?open=true")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    let resp = test::call_service(&app, open).await;
    let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(page["total"], 0);

    // But is still listed unfiltered, and by kind.
    let all = test::TestRequest::get()
        .uri("/api/v1/sessions?kind=monitor")
        .insert_header(("Authorization", bearer))
        .to_request();
    let resp = test::call_service(&app, all).await;
    let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(page["total"], 1);
}

#[actix_web::test]
async fn api_session_detail_resolves_the_operator_identity() {
    use sea_orm::{ActiveModelTrait, IntoActiveModel, Set};

    let state = test_state().await;
    let user = seed_user(&state.db, "op@example.com").await;
    let (raw, _) = seed_token(&state.db, user.id).await;

    // Give the operator a default station identity.
    let mut active = user.clone().into_active_model();
    active.callsign = Set(Some("W4USR".to_string()));
    active.grid = Set(Some("FM07".to_string()));
    active.update(&state.db).await.unwrap();

    let session = seed_session(&state.db, user.id, "run-1").await;
    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    // The session sets neither, so both fall back to the profile.
    let req = test::TestRequest::get()
        .uri(&format!("/api/v1/sessions/{}", session.id))
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
    let detail: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(detail["effective_callsign"], "W4USR");
    assert_eq!(detail["effective_grid"], "FM07");

    // A club call for this run overrides the default.
    let patch = test::TestRequest::patch()
        .uri("/api/v1/sessions/by-key/run-1")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(serde_json::json!({"client_key": "run-1", "operator_callsign": "W4CLUB"}))
        .to_request();
    assert_eq!(test::call_service(&app, patch).await.status(), 200);

    let req = test::TestRequest::get()
        .uri(&format!("/api/v1/sessions/{}", session.id))
        .insert_header(("Authorization", bearer))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let detail: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(detail["effective_callsign"], "W4CLUB");
    assert_eq!(detail["effective_grid"], "FM07", "grid still falls back");
}

#[actix_web::test]
async fn api_observations_batch_ingest_and_replay() {
    let state = test_state().await;
    let user = seed_user(&state.db, "op@example.com").await;
    let (raw, _) = seed_token(&state.db, user.id).await;
    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    let items = serde_json::json!([
        {"client_key": "c1", "callsign": "kr4nrc", "heard_at": "2026-07-24T23:12:41Z"},
        {"client_key": "c2", "callsign": "ve3xyz", "heard_at": "2026-07-24T23:14:02Z"},
        {"client_key": "c3", "callsign": "!!",     "heard_at": "2026-07-24T23:15:00Z"},
    ]);

    let req = test::TestRequest::post()
        .uri("/api/v1/observations")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(batch("run-1", items.clone()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200, "partial success is a 200, not a 201");
    let out: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(out["accepted"], 2);
    assert_eq!(out["duplicates"], 0);
    assert_eq!(out["stations_touched"], 2);
    assert_eq!(out["rejected"][0]["client_key"], "c3");

    // Replaying the identical batch must not duplicate anything — this is the property
    // the client's retry loop depends on.
    let again = test::TestRequest::post()
        .uri("/api/v1/observations")
        .insert_header(("Authorization", bearer.clone()))
        .set_json(batch("run-1", items))
        .to_request();
    let resp = test::call_service(&app, again).await;
    let out: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(out["accepted"], 0);
    assert_eq!(out["duplicates"], 2);

    // The roster now holds one row per distinct station, with the country resolved and
    // the band inherited from the session.
    let stations = test::TestRequest::get()
        .uri("/api/v1/stations?order=callsign")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    let resp = test::call_service(&app, stations).await;
    let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(page["total"], 2);
    assert_eq!(page["items"][0]["callsign"], "KR4NRC");
    assert_eq!(page["items"][0]["times_heard"], 1);
    assert_eq!(page["items"][0]["times_worked"], 0);
    assert_eq!(page["items"][1]["country"], "Canada");

    let obs = test::TestRequest::get()
        .uri("/api/v1/observations?callsign=KR4NRC")
        .insert_header(("Authorization", bearer))
        .to_request();
    let resp = test::call_service(&app, obs).await;
    let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["band"], "2m");
    assert!(
        page["items"][0]["transcript"].is_null(),
        "opt-in default is off"
    );
}

#[actix_web::test]
async fn api_observations_rejects_an_oversized_batch() {
    let state = test_state().await;
    let user = seed_user(&state.db, "op@example.com").await;
    let (raw, _) = seed_token(&state.db, user.id).await;
    let app = test_app!(state);

    let items: Vec<serde_json::Value> = (0..=crate::services::observations::MAX_BATCH)
        .map(|i| {
            serde_json::json!({
                "client_key": format!("c{i}"),
                "callsign": "W1AW",
                "heard_at": "2026-07-24T23:12:41Z",
            })
        })
        .collect();

    let req = test::TestRequest::post()
        .uri("/api/v1/observations")
        .insert_header(("Authorization", format!("Bearer {raw}")))
        .set_json(batch("run-1", serde_json::Value::Array(items)))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 400);
}

#[actix_web::test]
async fn api_observations_paginate() {
    let state = test_state().await;
    let user = seed_user(&state.db, "op@example.com").await;
    let (raw, _) = seed_token(&state.db, user.id).await;
    let session = seed_session(&state.db, user.id, "run-1").await;
    for call in ["W1AW", "K4CQ", "VE3XYZ", "W4ABC", "KR4NRC"] {
        seed_observation(&state.db, user.id, session.id, call).await;
    }
    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    let req = test::TestRequest::get()
        .uri("/api/v1/observations?limit=2&offset=2")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(page["total"], 5);
    assert_eq!(page["limit"], 2);
    assert_eq!(page["offset"], 2);
    assert_eq!(page["items"].as_array().unwrap().len(), 2);

    // An oversized limit is clamped, not rejected.
    let req = test::TestRequest::get()
        .uri("/api/v1/observations?limit=99999")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(page["limit"], crate::api::MAX_LIMIT);

    // An offset past the end is an empty page, not an error.
    let req = test::TestRequest::get()
        .uri("/api/v1/observations?offset=500")
        .insert_header(("Authorization", bearer))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert!(page["items"].as_array().unwrap().is_empty());
    assert_eq!(page["total"], 5);
}

#[actix_web::test]
async fn api_promote_observation_is_idempotent() {
    let state = test_state().await;
    let user = seed_user(&state.db, "op@example.com").await;
    let (raw, _) = seed_token(&state.db, user.id).await;
    let session = seed_session(&state.db, user.id, "run-1").await;
    let observation = seed_observation(&state.db, user.id, session.id, "KR4NRC").await;
    seed_station(&state.db, user.id, "KR4NRC").await;
    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    let promote = test::TestRequest::post()
        .uri(&format!("/api/v1/observations/{}/promote", observation.id))
        .insert_header(("Authorization", bearer.clone()))
        .set_json(serde_json::json!({"rst_sent": "59", "rst_received": "57"}))
        .to_request();
    let resp = test::call_service(&app, promote).await;
    assert_eq!(resp.status(), 201);
    let contact: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(contact["callsign"], "KR4NRC");
    assert_eq!(contact["rst_sent"], "59");

    // Promoting again returns the same contact rather than creating a second QSO.
    let again = test::TestRequest::post()
        .uri(&format!("/api/v1/observations/{}/promote", observation.id))
        .insert_header(("Authorization", bearer.clone()))
        .set_json(serde_json::json!({}))
        .to_request();
    let resp = test::call_service(&app, again).await;
    assert_eq!(resp.status(), 200);
    let repeat: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(repeat["id"], contact["id"]);

    // The station roster now derives times_worked from the logbook.
    let station = test::TestRequest::get()
        .uri("/api/v1/stations/kr4nrc")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    let resp = test::call_service(&app, station).await;
    let row: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(row["times_worked"], 1);

    let missing = test::TestRequest::post()
        .uri("/api/v1/observations/999999/promote")
        .insert_header(("Authorization", bearer))
        .set_json(serde_json::json!({}))
        .to_request();
    assert_eq!(test::call_service(&app, missing).await.status(), 404);
}

#[actix_web::test]
async fn api_monitoring_is_scoped_to_the_token_owner() {
    let state = test_state().await;
    let mine = seed_user(&state.db, "mine@example.com").await;
    let theirs = seed_user(&state.db, "theirs@example.com").await;
    let (raw, _) = seed_token(&state.db, mine.id).await;

    let their_session = seed_session(&state.db, theirs.id, "their-run").await;
    seed_observation(&state.db, theirs.id, their_session.id, "W1AW").await;
    seed_station(&state.db, theirs.id, "W1AW").await;

    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    for uri in [
        "/api/v1/sessions",
        "/api/v1/observations",
        "/api/v1/stations",
    ] {
        let req = test::TestRequest::get()
            .uri(uri)
            .insert_header(("Authorization", bearer.clone()))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
        assert_eq!(page["total"], 0, "{uri} leaked another user's rows");
    }

    let their_row = test::TestRequest::get()
        .uri(&format!("/api/v1/sessions/{}", their_session.id))
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    assert_eq!(test::call_service(&app, their_row).await.status(), 404);

    let their_station = test::TestRequest::get()
        .uri("/api/v1/stations/W1AW")
        .insert_header(("Authorization", bearer))
        .to_request();
    assert_eq!(test::call_service(&app, their_station).await.status(), 404);
}

#[actix_web::test]
async fn api_monitoring_requires_a_token() {
    let state = test_state().await;
    let app = test_app!(state);

    for uri in [
        "/api/v1/sessions",
        "/api/v1/observations",
        "/api/v1/stations",
    ] {
        let req = test::TestRequest::get().uri(uri).to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 401, "{uri}");
    }

    let req = test::TestRequest::post()
        .uri("/api/v1/observations")
        .set_json(batch("run-1", serde_json::json!([])))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), 401);
}

#[actix_web::test]
async fn api_stations_search_by_prefix() {
    let state = test_state().await;
    let user = seed_user(&state.db, "op@example.com").await;
    let (raw, _) = seed_token(&state.db, user.id).await;
    seed_station(&state.db, user.id, "KR4NRC").await;
    seed_station(&state.db, user.id, "K4CQ").await;
    seed_station(&state.db, user.id, "W1AW").await;
    let app = test_app!(state);
    let bearer = format!("Bearer {raw}");

    // Lowercase input is normalized before matching.
    let req = test::TestRequest::get()
        .uri("/api/v1/stations?q=k4")
        .insert_header(("Authorization", bearer.clone()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["callsign"], "K4CQ");

    // A blank query is ignored rather than matching nothing.
    let req = test::TestRequest::get()
        .uri("/api/v1/stations?q=%20&order=times_heard")
        .insert_header(("Authorization", bearer))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let page: serde_json::Value = serde_json::from_slice(&test::read_body(resp).await).unwrap();
    assert_eq!(page["total"], 3);
}

// ---------------------------------------------------------------------------
// Config parsing (env-mutating → serialized behind a lock)
// ---------------------------------------------------------------------------

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
const ENV_KEYS: &[&str] = &[
    "BIND_ADDRESS",
    "BASE_URL",
    "DATABASE_URL",
    "STATIC_DIR",
    "COOKIE_SECURE",
    "AUTH0_ROLES_CLAIM",
    "SESSION_SECRET",
    "AUTH0_DOMAIN",
    "AUTH0_CLIENT_ID",
    "AUTH0_CLIENT_SECRET",
];

fn clear_env() {
    for k in ENV_KEYS {
        // SAFETY: tests holding ENV_LOCK are the only ones mutating these vars.
        unsafe { std::env::remove_var(k) };
    }
}

fn set_env(k: &str, v: &str) {
    unsafe { std::env::set_var(k, v) };
}

#[actix_web::test]
async fn config_defaults() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    let c = Config::from_env().unwrap();
    assert_eq!(c.bind_address, "127.0.0.1:8080");
    assert_eq!(c.base_url, "http://localhost:8080");
    assert_eq!(c.static_dir, "crates/web/static");
    assert!(!c.cookie_secure);
    assert_eq!(c.roles_claim, "https://amateur-radio-tools/roles");
    assert!(c.session_secret.is_none());
    assert!(c.auth0.is_none());
    assert_eq!(c.redirect_uri(), "http://localhost:8080/auth/callback");
    assert!(c.summary().contains("auth0=<disabled>"));
}

#[actix_web::test]
async fn config_session_secret_rules() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    set_env("SESSION_SECRET", "too-short");
    assert!(Config::from_env().is_err());

    clear_env();
    let long = "a".repeat(64);
    set_env("SESSION_SECRET", &long);
    assert_eq!(
        Config::from_env().unwrap().session_secret.as_deref(),
        Some(long.as_str())
    );
    clear_env();
}

#[actix_web::test]
async fn config_auth0_and_bools() {
    let _guard = ENV_LOCK.lock().unwrap();
    clear_env();
    // Only a domain ⇒ Auth0 stays disabled (all-or-nothing).
    set_env("AUTH0_DOMAIN", "tenant.us.auth0.com");
    assert!(Config::from_env().unwrap().auth0.is_none());

    // All three ⇒ enabled, domain normalized (scheme + trailing slash stripped).
    set_env("AUTH0_DOMAIN", "https://tenant.us.auth0.com/");
    set_env("AUTH0_CLIENT_ID", "cid");
    set_env("AUTH0_CLIENT_SECRET", "secret");
    set_env("COOKIE_SECURE", "yes");
    let c = Config::from_env().unwrap();
    let a = c.auth0.expect("auth0 enabled");
    assert_eq!(a.domain, "tenant.us.auth0.com");
    assert!(c.cookie_secure);

    clear_env();
    set_env("COOKIE_SECURE", "0");
    assert!(!Config::from_env().unwrap().cookie_secure);
    clear_env();
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn app_error_status_codes() {
    use actix_web::http::StatusCode;
    assert_eq!(AppError::NotFound.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(AppError::Forbidden.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(
        AppError::BadRequest("x".into()).status_code(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        AppError::AuthNotConfigured.status_code(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        AppError::Auth("x".into()).status_code(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        AppError::Internal(anyhow::anyhow!("boom")).status_code(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    // From<DbErr>
    let e: AppError = sea_orm::DbErr::Custom("db".into()).into();
    assert_eq!(e.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[actix_web::test]
async fn app_error_renders_html() {
    let resp = AppError::NotFound.error_response();
    assert_eq!(resp.status(), 404);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.contains("text/html"));
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("404"));
}

#[actix_web::test]
async fn api_error_renders_json() {
    use actix_web::http::StatusCode;
    assert_eq!(
        ApiError::Unauthorized.status_code(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ApiError::BadRequest("x".into()).status_code(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(ApiError::NotFound.status_code(), StatusCode::NOT_FOUND);
    assert_eq!(
        ApiError::internal("x").status_code(),
        StatusCode::INTERNAL_SERVER_ERROR
    );

    let resp = ApiError::Unauthorized.error_response();
    assert_eq!(resp.status(), 401);
    assert!(resp.headers().contains_key(header::WWW_AUTHENTICATE));
    let body = actix_web::body::to_bytes(resp.into_body()).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"], "unauthorized");

    // From<DbErr> maps to internal.
    let e: ApiError = sea_orm::DbErr::Custom("db".into()).into();
    assert_eq!(e.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

// ---------------------------------------------------------------------------
// Session helpers
// ---------------------------------------------------------------------------

#[actix_web::test]
async fn session_user_helpers() {
    let named = session_user(1, vec!["Admin".to_string()]);
    assert_eq!(named.display_name(), "Test User");
    assert_eq!(named.initials(), "TU");
    assert!(named.has_role("admin")); // case-insensitive
    assert!(named.is_admin());

    let no_name = SessionUser {
        id: 2,
        sub: "s".into(),
        email: "solo@example.com".into(),
        name: None,
        picture: None,
        roles: vec![],
    };
    assert_eq!(no_name.display_name(), "solo@example.com");
    assert_eq!(no_name.initials(), "S");
    assert!(!no_name.is_admin());
}
