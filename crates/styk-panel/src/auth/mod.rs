pub mod limiter;
pub mod password;
pub mod telegram;
pub mod totp;

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Duration, Utc};

use crate::crypto::{ct_eq, random_token, sha256_hex};
use crate::error::ApiError;
use crate::state::AppState;

pub const CSRF_COOKIE: &str = "styk_csrf";
pub const CSRF_HEADER: &str = "x-csrf-token";
pub const SESSION_TTL: Duration = Duration::days(7);
pub const SESSION_IDLE: Duration = Duration::hours(12);
pub const PENDING_TTL: Duration = Duration::minutes(10);
/// Sudo is an idle timeout: every mutating request made while it is active extends it.
pub const SUDO_TTL: Duration = Duration::minutes(30);

pub fn session_cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-styk_session"
    } else {
        "styk_session"
    }
}

pub fn session_cookie(state: &AppState, token: String) -> Cookie<'static> {
    Cookie::build((session_cookie_name(state.cfg.cookie_secure), token))
        .path("/")
        .http_only(true)
        .secure(state.cfg.cookie_secure)
        .same_site(SameSite::Strict)
        .build()
}

/// Session row as seen by handlers; `stage` tells whether sign-in finished.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Session {
    pub token_hash: String,
    pub user_id: i64,
    pub username: String,
    pub stage: String,
    pub method: String,
    pub sudo_until: Option<DateTime<Utc>>,
    pub totp_enabled: bool,
    pub totp_secret_enc: Option<Vec<u8>>,
}

impl Session {
    pub async fn load(state: &AppState, jar: &CookieJar) -> Result<Option<Session>, ApiError> {
        let Some(c) = jar.get(session_cookie_name(state.cfg.cookie_secure)) else {
            return Ok(None);
        };
        let row = sqlx::query_as::<_, Session>(
            "SELECT s.token_hash, s.user_id, u.username, s.stage, s.method, s.sudo_until,
                    u.totp_enabled, u.totp_secret_enc
               FROM sessions s JOIN users u ON u.id = s.user_id
              WHERE s.token_hash = $1 AND s.expires_at > now() AND NOT u.disabled
                AND (s.stage <> 'active' OR s.last_seen > now() - $2::interval)",
        )
        .bind(sha256_hex(c.value()))
        .bind(format!("{} seconds", SESSION_IDLE.num_seconds()))
        .fetch_optional(&state.db)
        .await?;
        Ok(row)
    }

    pub fn totp_secret(&self, state: &AppState) -> Result<Option<String>, ApiError> {
        self.totp_secret_enc
            .as_deref()
            .map(|enc| {
                Ok(String::from_utf8(state.cipher.decrypt(enc)?).map_err(anyhow::Error::from)?)
            })
            .transpose()
    }
}

/// An authenticated, fully signed-in user. Rejects pending sessions.
#[derive(Debug, Clone)]
pub struct AuthUser(pub Session);

impl std::ops::Deref for AuthUser {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.0
    }
}

impl AuthUser {
    pub fn require_sudo(&self) -> Result<(), ApiError> {
        match self.sudo_until {
            Some(t) if t > Utc::now() => Ok(()),
            _ => Err(ApiError::sudo_required()),
        }
    }
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let jar = CookieJar::from_headers(&parts.headers);
        let s = Session::load(state, &jar)
            .await?
            .ok_or(ApiError::Unauthorized)?;
        if s.stage != "active" {
            return Err(ApiError::Unauthorized);
        }
        sqlx::query("UPDATE sessions SET last_seen = now() WHERE token_hash = $1 AND last_seen < now() - interval '1 minute'")
            .bind(&s.token_hash)
            .execute(&state.db)
            .await?;
        let mut s = s;
        // Only actions keep sudo alive; background polling (GET) must not.
        if parts.method != Method::GET && s.sudo_until.is_some_and(|t| t > Utc::now()) {
            s.sudo_until = Some(sqlx::query_scalar(
                "UPDATE sessions SET sudo_until = now() + $2::interval WHERE token_hash = $1 RETURNING sudo_until",
            )
            .bind(&s.token_hash)
            .bind(format!("{} seconds", SUDO_TTL.num_seconds()))
            .fetch_one(&state.db)
            .await?);
        }
        Ok(AuthUser(s))
    }
}

/// Client IP and user agent, honouring the proxy headers only when configured.
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub ip: String,
    pub user_agent: Option<String>,
}

impl FromRequestParts<AppState> for ClientInfo {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = |name: &str| {
            parts
                .headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
        };
        let proxied = state
            .cfg
            .trust_proxy
            .then(|| {
                header("cf-connecting-ip").or_else(|| header("x-forwarded-for")?.split(',').next())
            })
            .flatten()
            .map(|s| s.trim().to_string());
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip().to_string());
        Ok(ClientInfo {
            ip: proxied.or(peer).unwrap_or_else(|| "unknown".into()),
            user_agent: header("user-agent").map(|s| s.chars().take(300).collect()),
        })
    }
}

/// Double-submit CSRF check for state-changing API calls; also hands out the cookie.
pub async fn csrf(
    State(state): State<AppState>,
    jar: CookieJar,
    req: Request,
    next: Next,
) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    // Nested under /api, so the router sees the stripped path.
    let path = req.uri().path();
    let exempt = path.starts_with("/hooks/") || path.starts_with("/api/hooks/");
    let cookie = jar.get(CSRF_COOKIE).map(|c| c.value().to_string());
    if !safe && !exempt {
        let header = req.headers().get(CSRF_HEADER).and_then(|v| v.to_str().ok());
        match (&cookie, header) {
            (Some(c), Some(h)) if ct_eq(c, h) => {}
            _ => {
                return ApiError::Forbidden("csrf", "missing or invalid CSRF token".into())
                    .into_response()
            }
        }
    }
    let mut resp = next.run(req).await;
    if cookie.is_none() {
        let c = Cookie::build((CSRF_COOKIE, random_token()))
            .path("/")
            .secure(state.cfg.cookie_secure)
            .same_site(SameSite::Strict)
            .build();
        if let Ok(v) = HeaderValue::from_str(&c.to_string()) {
            resp.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    resp
}
