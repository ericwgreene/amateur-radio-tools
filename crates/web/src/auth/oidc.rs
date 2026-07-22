//! Auth0 integration via OpenID Connect (Authorization Code flow with PKCE).
//!
//! Auth0 is a standard OIDC provider, so we use the `openidconnect` crate against its
//! discovery document (`https://<domain>/.well-known/openid-configuration`) rather than
//! hand-rolling the protocol.
//!
//! ## A note on the client type
//!
//! `openidconnect` v4 encodes *which endpoints are configured* in the client's type
//! (a "typestate"), which makes the fully-configured client type awkward to name in a
//! struct field. We sidestep that by storing only the discovered provider metadata and
//! credentials, and constructing the client as a local variable inside each method — its
//! type is then inferred and never needs to be written down. Building the client is cheap
//! (no network I/O; discovery already happened once at startup).

use crate::auth::session::AuthFlow;
use crate::config::{Auth0Config, Config};
use crate::error::AppError;

use anyhow::{Context, Result};
use base64::Engine;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreIdToken, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use url::Url;

/// A verified identity extracted from the Auth0 ID token.
#[derive(Clone, Debug)]
pub struct VerifiedIdentity {
    pub sub: String,
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub roles: Vec<String>,
}

pub struct AuthClient {
    provider_metadata: CoreProviderMetadata,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    domain: String,
    roles_claim: String,
    http: reqwest::Client,
}

impl AuthClient {
    /// Perform OIDC discovery against the Auth0 tenant and build the client.
    pub async fn discover(
        a0: &Auth0Config,
        config: &Config,
        http: reqwest::Client,
    ) -> Result<Self> {
        let issuer = IssuerUrl::new(format!("https://{}/", a0.domain))
            .context("invalid Auth0 issuer URL")?;
        let provider_metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .context("OIDC discovery request to Auth0 failed")?;

        Ok(Self {
            provider_metadata,
            client_id: a0.client_id.clone(),
            client_secret: a0.client_secret.clone(),
            redirect_uri: config.redirect_uri(),
            domain: a0.domain.clone(),
            roles_claim: config.roles_claim.clone(),
            http,
        })
    }

    /// Begin login: returns the Auth0 authorize URL to redirect the browser to, plus the
    /// [`AuthFlow`] state that must be stashed in the session for the callback.
    pub fn begin_login(&self, return_to: Option<String>) -> Result<(Url, AuthFlow)> {
        // Construct the client as a local: its (typestate) type is inferred, never named.
        let client = CoreClient::from_provider_metadata(
            self.provider_metadata.clone(),
            ClientId::new(self.client_id.clone()),
            Some(ClientSecret::new(self.client_secret.clone())),
        )
        .set_redirect_uri(
            RedirectUrl::new(self.redirect_uri.clone()).context("invalid redirect URI")?,
        );

        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

        let (auth_url, csrf, nonce) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .add_scope(Scope::new("openid".to_string()))
            .add_scope(Scope::new("profile".to_string()))
            .add_scope(Scope::new("email".to_string()))
            .set_pkce_challenge(pkce_challenge)
            .url();

        let flow = AuthFlow {
            pkce_verifier: pkce_verifier.secret().clone(),
            nonce: nonce.secret().clone(),
            csrf_state: csrf.secret().clone(),
            return_to,
        };
        Ok((auth_url, flow))
    }

    /// Complete login: exchange the authorization `code` for tokens, verify the ID token
    /// against the stored nonce, and extract the user's identity and roles.
    pub async fn complete_login(
        &self,
        code: String,
        flow: &AuthFlow,
    ) -> std::result::Result<VerifiedIdentity, AppError> {
        let client = CoreClient::from_provider_metadata(
            self.provider_metadata.clone(),
            ClientId::new(self.client_id.clone()),
            Some(ClientSecret::new(self.client_secret.clone())),
        )
        .set_redirect_uri(
            RedirectUrl::new(self.redirect_uri.clone())
                .map_err(|e| AppError::Auth(format!("invalid redirect URI: {e}")))?,
        );

        let token_response = client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|e| AppError::Auth(format!("token endpoint is not configured: {e}")))?
            .set_pkce_verifier(PkceCodeVerifier::new(flow.pkce_verifier.clone()))
            .request_async(&self.http)
            .await
            .map_err(|e| AppError::Auth(format!("token exchange failed: {e}")))?;

        let id_token = token_response
            .id_token()
            .ok_or_else(|| AppError::Auth("Auth0 did not return an ID token".to_string()))?;

        let nonce = Nonce::new(flow.nonce.clone());
        let verifier = client.id_token_verifier();
        let claims = id_token
            .claims(&verifier, &nonce)
            .map_err(|e| AppError::Auth(format!("ID token verification failed: {e}")))?;

        let sub = claims.subject().as_str().to_string();
        let email = claims
            .email()
            .map(|e| e.as_str().to_string())
            .unwrap_or_default();
        let name = claims
            .name()
            .and_then(|n| n.get(None))
            .map(|n| n.as_str().to_string());
        let picture = claims
            .picture()
            .and_then(|p| p.get(None))
            .map(|p| p.as_str().to_string());

        let roles = extract_roles(id_token, &self.roles_claim);

        Ok(VerifiedIdentity {
            sub,
            email,
            name,
            picture,
            roles,
        })
    }

    /// Build the Auth0 logout URL, which clears the Auth0 session and redirects the
    /// browser back to `return_to` (which must be registered as an Allowed Logout URL).
    pub fn logout_url(&self, return_to: &str) -> String {
        let mut url = Url::parse(&format!("https://{}/v2/logout", self.domain))
            .expect("logout URL is always valid");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("returnTo", return_to);
        url.to_string()
    }
}

/// Extract the roles array from a namespaced custom claim in the ID token.
///
/// Auth0 delivers roles via a custom claim (populated by a post-login Action, see README)
/// under a namespaced key such as `https://amateur-radio-tools/roles`. The `openidconnect`
/// `CoreClient` only surfaces the standard claims, so we read this one straight from the
/// token payload. This is safe because the caller has *already* cryptographically verified
/// the ID token (signature, issuer, audience, expiry, nonce) before we get here — we are
/// only re-reading the payload of a token we have proven to be authentic.
fn extract_roles(id_token: &CoreIdToken, claim: &str) -> Vec<String> {
    // `IdToken` serializes to its compact JWT string form.
    let compact = match serde_json::to_value(id_token) {
        Ok(serde_json::Value::String(s)) => s,
        _ => return Vec::new(),
    };
    let Some(payload_b64) = compact.split('.').nth(1) else {
        return Vec::new();
    };
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload_b64) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    match json.get(claim) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}
