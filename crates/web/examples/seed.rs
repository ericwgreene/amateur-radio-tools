//! Dev-only seeding tool.
//!
//! Creates (or reuses) a local user and mints an API token, so you can exercise the REST
//! API locally without configuring Auth0. Prints the raw token — the same value is not
//! recoverable afterwards.
//!
//! Usage:
//!   cargo run -p web --example seed
//!   DATABASE_URL=sqlite://./data/app.db?mode=rwc SEED_EMAIL=me@example.com \
//!       cargo run -p web --example seed
//!
//! Then, using the printed token:
//!   curl -H "Authorization: Bearer <token>" http://localhost:8080/api/v1/me

use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use sea_orm::{ActiveModelTrait, ColumnTrait, Database, EntityTrait, QueryFilter, Set};
use sha2::{Digest, Sha256};

use entity::{api_tokens, users};
use migration::{Migrator, MigratorTrait};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://./data/amateur_radio_tools.db?mode=rwc".to_string());
    let email = std::env::var("SEED_EMAIL").unwrap_or_else(|_| "dev@example.com".to_string());

    let db = Database::connect(&db_url).await?;
    Migrator::up(&db, None).await?;

    let now = Utc::now();
    let sub = format!("dev|{email}");

    // Reuse an existing dev user, or create one.
    let user = match users::Entity::find()
        .filter(users::Column::Auth0Sub.eq(&sub))
        .one(&db)
        .await?
    {
        Some(u) => u,
        None => {
            users::ActiveModel {
                auth0_sub: Set(sub),
                email: Set(email.clone()),
                name: Set(Some("Dev User".to_string())),
                picture: Set(None),
                created_at: Set(now),
                updated_at: Set(now),
                last_login_at: Set(Some(now)),
                ..Default::default()
            }
            .insert(&db)
            .await?
        }
    };

    // Mint a token (same shape as the app: `art_` + base64url(32 random bytes)).
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let raw = format!(
        "art_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    );
    let hash = {
        let digest = Sha256::digest(raw.as_bytes());
        let mut hex = String::with_capacity(64);
        for b in digest {
            hex.push_str(&format!("{b:02x}"));
        }
        hex
    };
    let prefix: String = raw.chars().take(12).collect();

    api_tokens::ActiveModel {
        user_id: Set(user.id),
        name: Set("dev seed token".to_string()),
        token_hash: Set(hash),
        token_prefix: Set(prefix),
        created_at: Set(now),
        last_used_at: Set(None),
        revoked_at: Set(None),
        ..Default::default()
    }
    .insert(&db)
    .await?;

    println!("seeded user id={} email={}", user.id, email);
    println!("API_TOKEN={raw}");
    Ok(())
}
