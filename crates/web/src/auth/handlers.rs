//! HTTP handlers for the OIDC login flow: `/login`, `/auth/callback`, `/logout`.

use actix_session::Session;
use actix_web::{HttpResponse, get, http::header::LOCATION, web};
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde::Deserialize;

use crate::auth::session::{AuthFlow, SESSION_FLOW_KEY, SESSION_USER_KEY, SessionUser};
use crate::error::AppError;
use crate::state::AppState;
use entity::users;

#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    /// Optional relative path to return to after login (must start with `/`).
    return_to: Option<String>,
}

/// Kick off the Authorization Code flow: build the Auth0 authorize URL, stash the PKCE
/// verifier / nonce / CSRF state in the session, and redirect the browser to Auth0.
#[get("/login")]
pub async fn login(
    state: web::Data<AppState>,
    session: Session,
    query: web::Query<LoginQuery>,
) -> Result<HttpResponse, AppError> {
    let auth = state.auth.as_ref().ok_or(AppError::AuthNotConfigured)?;

    let return_to = query.return_to.clone().filter(|r| r.starts_with('/'));
    let (auth_url, flow) = auth.begin_login(return_to).map_err(AppError::Internal)?;

    session
        .insert(SESSION_FLOW_KEY, &flow)
        .map_err(|e| AppError::Internal(e.into()))?;

    Ok(HttpResponse::SeeOther()
        .insert_header((LOCATION, auth_url.to_string()))
        .finish())
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Auth0 redirects the browser here with an authorization `code` (or an `error`). We
/// validate the CSRF `state`, exchange the code for tokens, verify the ID token, upsert
/// the local user record, and establish the session.
#[get("/auth/callback")]
pub async fn callback(
    state: web::Data<AppState>,
    session: Session,
    query: web::Query<CallbackQuery>,
) -> Result<HttpResponse, AppError> {
    let auth = state.auth.as_ref().ok_or(AppError::AuthNotConfigured)?;

    // Auth0 reported an error (e.g. the user declined consent).
    if let Some(error) = &query.error {
        let description = query.error_description.clone().unwrap_or_default();
        return Err(AppError::Auth(format!("{error}: {description}")));
    }

    let code = query
        .code
        .clone()
        .ok_or_else(|| AppError::BadRequest("missing authorization code".to_string()))?;
    let returned_state = query
        .state
        .clone()
        .ok_or_else(|| AppError::BadRequest("missing state parameter".to_string()))?;

    // Recover and consume the in-flight login state.
    let flow: AuthFlow = session
        .get(SESSION_FLOW_KEY)
        .ok()
        .flatten()
        .ok_or_else(|| {
            AppError::Auth("no login is in progress (did the session expire?)".to_string())
        })?;
    session.remove(SESSION_FLOW_KEY);

    // CSRF protection: the state Auth0 echoes back must match what we generated.
    if returned_state != flow.csrf_state {
        return Err(AppError::Auth("state mismatch (possible CSRF)".to_string()));
    }

    let identity = auth.complete_login(code, &flow).await?;

    // Upsert the local user mirror, keyed by the Auth0 subject.
    let now = Utc::now();
    let existing = users::Entity::find()
        .filter(users::Column::Auth0Sub.eq(&identity.sub))
        .one(&state.db)
        .await?;

    let user = match existing {
        Some(model) => {
            let mut active = model.into_active_model();
            active.email = Set(identity.email.clone());
            active.name = Set(identity.name.clone());
            active.picture = Set(identity.picture.clone());
            active.updated_at = Set(now);
            active.last_login_at = Set(Some(now));
            active.update(&state.db).await?
        }
        None => {
            users::ActiveModel {
                auth0_sub: Set(identity.sub.clone()),
                email: Set(identity.email.clone()),
                name: Set(identity.name.clone()),
                picture: Set(identity.picture.clone()),
                created_at: Set(now),
                updated_at: Set(now),
                last_login_at: Set(Some(now)),
                ..Default::default()
            }
            .insert(&state.db)
            .await?
        }
    };

    let session_user = SessionUser {
        id: user.id,
        sub: identity.sub,
        email: identity.email,
        name: identity.name,
        picture: identity.picture,
        roles: identity.roles,
    };
    session
        .insert(SESSION_USER_KEY, &session_user)
        .map_err(|e| AppError::Internal(e.into()))?;

    let destination = flow
        .return_to
        .filter(|r| r.starts_with('/'))
        .unwrap_or_else(|| "/dashboard".to_string());

    Ok(HttpResponse::SeeOther()
        .insert_header((LOCATION, destination))
        .finish())
}

/// Clear the local session and, if Auth0 is configured, end the Auth0 session too
/// (redirecting back to the site afterwards).
#[get("/logout")]
pub async fn logout(
    state: web::Data<AppState>,
    session: Session,
) -> Result<HttpResponse, AppError> {
    session.purge();

    let destination = match state.auth.as_ref() {
        Some(auth) => auth.logout_url(&state.config.base_url),
        None => "/".to_string(),
    };

    Ok(HttpResponse::SeeOther()
        .insert_header((LOCATION, destination))
        .finish())
}
