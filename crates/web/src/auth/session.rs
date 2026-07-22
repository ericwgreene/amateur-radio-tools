//! Session-backed user identity and request extractors.
//!
//! Two extractors are provided:
//!   * [`MaybeUser`] — always succeeds, yielding `Some(user)` when logged in. Use it on
//!     public pages that render differently for signed-in visitors.
//!   * [`AuthedUser`] — requires a logged-in user, otherwise short-circuits the request
//!     with a redirect to `/login` (or an `HX-Redirect` header for HTMX requests).

use std::future::{Ready, ready};

use actix_session::SessionExt;
use actix_web::{FromRequest, HttpRequest, HttpResponse, dev::Payload, http::header};
use serde::{Deserialize, Serialize};

/// Session key under which the authenticated user is stored.
pub const SESSION_USER_KEY: &str = "user";
/// Session key holding the in-flight OIDC login state (PKCE verifier, nonce, CSRF state).
pub const SESSION_FLOW_KEY: &str = "auth_flow";

/// The authenticated user, as stored in the (encrypted) session cookie.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionUser {
    /// Local database id.
    pub id: i64,
    /// Auth0 subject (`sub`) claim.
    pub sub: String,
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub roles: Vec<String>,
}

impl SessionUser {
    /// A display name, falling back to the email address.
    pub fn display_name(&self) -> &str {
        match self.name.as_deref() {
            Some(n) if !n.is_empty() => n,
            _ => &self.email,
        }
    }

    /// Uppercase initials for avatar placeholders.
    pub fn initials(&self) -> String {
        self.display_name()
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase()
    }

    /// Case-insensitive role membership check.
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r.eq_ignore_ascii_case(role))
    }

    pub fn is_admin(&self) -> bool {
        self.has_role("admin")
    }
}

/// In-flight OIDC login state, persisted server-side (in the encrypted session cookie)
/// between the `/login` redirect and the `/auth/callback`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthFlow {
    pub pkce_verifier: String,
    pub nonce: String,
    pub csrf_state: String,
    pub return_to: Option<String>,
}

fn user_from_request(req: &HttpRequest) -> Option<SessionUser> {
    req.get_session()
        .get::<SessionUser>(SESSION_USER_KEY)
        .ok()
        .flatten()
}

/// Extractor yielding `Some(user)` when signed in, `None` otherwise. Never fails.
pub struct MaybeUser(pub Option<SessionUser>);

impl FromRequest for MaybeUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        ready(Ok(MaybeUser(user_from_request(req))))
    }
}

/// Extractor requiring an authenticated user. Redirects to `/login` if absent.
pub struct AuthedUser(pub SessionUser);

impl FromRequest for AuthedUser {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        match user_from_request(req) {
            Some(user) => ready(Ok(AuthedUser(user))),
            None => ready(Err(login_redirect(req))),
        }
    }
}

/// Build a redirect-to-login response. For HTMX requests we return `200` with an
/// `HX-Redirect` header so HTMX performs a full client-side navigation to the login page;
/// otherwise a normal `303 See Other`.
fn login_redirect(req: &HttpRequest) -> actix_web::Error {
    let is_htmx = req.headers().contains_key("HX-Request");
    let response = if is_htmx {
        HttpResponse::Ok()
            .insert_header(("HX-Redirect", "/login"))
            .finish()
    } else {
        HttpResponse::SeeOther()
            .insert_header((header::LOCATION, "/login"))
            .finish()
    };
    actix_web::error::InternalError::from_response("authentication required", response).into()
}
