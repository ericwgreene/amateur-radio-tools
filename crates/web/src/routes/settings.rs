//! User settings — currently API token management.
//!
//! Tokens are created here for use with the REST API. The raw token is displayed exactly
//! once (right after creation); afterwards only its prefix is shown.

use actix_web::{get, post, web};
use askama::Template;
use askama_web::WebTemplate;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Set,
};
use serde::Deserialize;

use crate::auth::api_token::generate_token;
use crate::auth::session::{AuthedUser, SessionUser};
use crate::error::AppError;
use crate::state::AppState;
use entity::api_tokens;

struct TokenView {
    id: i64,
    name: String,
    prefix: String,
    created_at: String,
    last_used: String,
    revoked: bool,
}

fn fmt_dt(dt: Option<chrono::DateTime<Utc>>) -> String {
    dt.map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "—".to_string())
}

async fn load_tokens(state: &AppState, user_id: i64) -> Result<Vec<TokenView>, AppError> {
    let rows = api_tokens::Entity::find()
        .filter(api_tokens::Column::UserId.eq(user_id))
        .order_by_desc(api_tokens::Column::CreatedAt)
        .all(&state.db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|t| TokenView {
            id: t.id,
            name: t.name,
            prefix: t.token_prefix,
            created_at: fmt_dt(Some(t.created_at)),
            last_used: fmt_dt(t.last_used_at),
            revoked: t.revoked_at.is_some(),
        })
        .collect())
}

#[derive(Template, WebTemplate)]
#[template(path = "settings_tokens.html")]
struct TokensPage {
    current_user: Option<SessionUser>,
    tokens: Vec<TokenView>,
    base_url: String,
}

/// Fragment: just the token table rows (used to refresh after revoke).
#[derive(Template, WebTemplate)]
#[template(path = "partials/token_rows.html")]
struct TokenRows {
    tokens: Vec<TokenView>,
}

/// Fragment shown once after creating a token: the secret banner plus an out-of-band
/// refresh of the token table.
#[derive(Template, WebTemplate)]
#[template(path = "partials/token_created.html")]
struct TokenCreated {
    raw_token: String,
    name: String,
    tokens: Vec<TokenView>,
}

#[get("/settings/tokens")]
pub async fn tokens_page(
    user: AuthedUser,
    state: web::Data<AppState>,
) -> Result<TokensPage, AppError> {
    let tokens = load_tokens(&state, user.0.id).await?;
    Ok(TokensPage {
        current_user: Some(user.0),
        tokens,
        base_url: state.config.base_url.clone(),
    })
}

#[derive(Debug, Deserialize)]
pub struct CreateTokenForm {
    name: Option<String>,
}

#[post("/settings/tokens")]
pub async fn create_token(
    user: AuthedUser,
    state: web::Data<AppState>,
    form: web::Form<CreateTokenForm>,
) -> Result<TokenCreated, AppError> {
    let name = form
        .into_inner()
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "API token".to_string());

    let token = generate_token();
    let now = Utc::now();

    api_tokens::ActiveModel {
        user_id: Set(user.0.id),
        name: Set(name.clone()),
        token_hash: Set(token.hash),
        token_prefix: Set(token.prefix),
        created_at: Set(now),
        last_used_at: Set(None),
        revoked_at: Set(None),
        ..Default::default()
    }
    .insert(&state.db)
    .await?;

    let tokens = load_tokens(&state, user.0.id).await?;
    Ok(TokenCreated {
        raw_token: token.raw,
        name,
        tokens,
    })
}

#[post("/settings/tokens/{id}/revoke")]
pub async fn revoke_token(
    user: AuthedUser,
    state: web::Data<AppState>,
    path: web::Path<i64>,
) -> Result<TokenRows, AppError> {
    let id = path.into_inner();

    // Only revoke a token that belongs to the current user.
    if let Some(token) = api_tokens::Entity::find_by_id(id)
        .filter(api_tokens::Column::UserId.eq(user.0.id))
        .one(&state.db)
        .await?
    {
        if token.revoked_at.is_none() {
            let mut active = token.into_active_model();
            active.revoked_at = Set(Some(Utc::now()));
            active.update(&state.db).await?;
        }
    }

    let tokens = load_tokens(&state, user.0.id).await?;
    Ok(TokenRows { tokens })
}
