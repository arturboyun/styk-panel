use axum::extract::State;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::audit::{self, Entry};
use crate::auth::{
    password, session_cookie, session_cookie_name, telegram, totp, AuthUser, ClientInfo, Session,
    PENDING_TTL, SESSION_TTL, SUDO_TTL,
};
use crate::crypto::{random_token, sha256_hex};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

#[derive(Serialize)]
pub struct AuthConfig {
    telegram_bot_username: Option<String>,
}

pub async fn config(State(st): State<AppState>) -> Json<AuthConfig> {
    Json(AuthConfig {
        telegram_bot_username: st
            .cfg
            .telegram_bot_token
            .as_ref()
            .and(st.cfg.telegram_bot_username.clone()),
    })
}

#[derive(Serialize)]
pub struct Me {
    id: i64,
    username: String,
    method: String,
    sudo_until: Option<DateTime<Utc>>,
}

pub async fn me(user: AuthUser) -> Json<Me> {
    Json(Me {
        id: user.user_id,
        username: user.username.clone(),
        method: user.method.clone(),
        sudo_until: user.sudo_until,
    })
}

async fn create_session(
    st: &AppState,
    jar: CookieJar,
    client: &ClientInfo,
    user_id: i64,
    stage: &str,
    method: &str,
) -> ApiResult<CookieJar> {
    let token = random_token();
    let ttl = if stage == "active" {
        SESSION_TTL
    } else {
        PENDING_TTL
    };
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, stage, method, ip, user_agent, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6, now() + $7::interval)",
    )
    .bind(sha256_hex(&token))
    .bind(user_id)
    .bind(stage)
    .bind(method)
    .bind(&client.ip)
    .bind(&client.user_agent)
    .bind(format!("{} seconds", ttl.num_seconds()))
    .execute(&st.db)
    .await?;
    Ok(jar.add(session_cookie(st, token)))
}

#[derive(Deserialize)]
pub struct LoginReq {
    username: String,
    password: String,
}

#[derive(Serialize)]
pub struct LoginResp {
    /// "totp" or "totp_setup"
    next: &'static str,
}

pub async fn login(
    State(st): State<AppState>,
    client: ClientInfo,
    jar: CookieJar,
    Json(req): Json<LoginReq>,
) -> ApiResult<(CookieJar, Json<LoginResp>)> {
    let username = req.username.trim();
    if st.limiter.blocked(&client.ip, Some(username)) {
        return Err(ApiError::TooManyRequests);
    }
    let user: Option<(i64, String, bool)> = sqlx::query_as(
        "SELECT id, password_hash, totp_enabled FROM users WHERE lower(username) = lower($1) AND NOT disabled",
    )
    .bind(username)
    .fetch_optional(&st.db)
    .await?;

    let ok_user = match &user {
        Some((id, hash, totp)) if password::verify(&req.password, hash) => Some((*id, *totp)),
        Some(_) => None,
        None => {
            password::verify_dummy(&req.password);
            None
        }
    };
    let Some((user_id, totp_enabled)) = ok_user else {
        st.limiter.fail(&client.ip, Some(username));
        audit::record(
            &st,
            Entry {
                user_id: user.map(|u| u.0),
                username: Some(username),
                action: "auth.login_failed",
                target: None,
                ip: &client.ip,
                ok: false,
                details: json!({ "method": "password" }),
            },
        )
        .await;
        return Err(ApiError::Forbidden(
            "invalid_credentials",
            "wrong username or password".into(),
        ));
    };
    let stage = if totp_enabled {
        "pending_totp"
    } else {
        "totp_setup"
    };
    let jar = create_session(&st, jar, &client, user_id, stage, "password").await?;
    Ok((
        jar,
        Json(LoginResp {
            next: if totp_enabled { "totp" } else { "totp_setup" },
        }),
    ))
}

async fn pending(st: &AppState, jar: &CookieJar) -> ApiResult<Session> {
    let s = Session::load(st, jar)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if s.stage == "active" {
        return Err(ApiError::BadRequest("already signed in".into()));
    }
    Ok(s)
}

#[derive(Serialize)]
pub struct TotpSetup {
    otpauth_url: String,
    secret: String,
}

pub async fn totp_setup(State(st): State<AppState>, jar: CookieJar) -> ApiResult<Json<TotpSetup>> {
    let s = pending(&st, &jar).await?;
    if s.stage != "totp_setup" {
        return Err(ApiError::BadRequest("TOTP is already configured".into()));
    }
    let secret = match s.totp_secret(&st)? {
        Some(sec) => sec,
        None => {
            let sec = totp::generate_secret();
            sqlx::query("UPDATE users SET totp_secret_enc = $1 WHERE id = $2 AND NOT totp_enabled")
                .bind(st.cipher.encrypt(sec.as_bytes())?)
                .bind(s.user_id)
                .execute(&st.db)
                .await?;
            sec
        }
    };
    Ok(Json(TotpSetup {
        otpauth_url: totp::otpauth_url(&secret, &s.username)?,
        secret,
    }))
}

#[derive(Deserialize)]
pub struct CodeReq {
    code: String,
}

#[derive(Serialize)]
pub struct TotpResp {
    recovery_codes: Option<Vec<String>>,
}

async fn use_recovery_code(st: &AppState, user_id: i64, code: &str) -> ApiResult<bool> {
    let norm: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    let res = sqlx::query(
        "UPDATE recovery_codes SET used_at = now() WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL",
    )
    .bind(user_id)
    .bind(sha256_hex(&norm))
    .execute(&st.db)
    .await?;
    Ok(res.rows_affected() == 1)
}

pub async fn totp_verify(
    State(st): State<AppState>,
    client: ClientInfo,
    jar: CookieJar,
    Json(req): Json<CodeReq>,
) -> ApiResult<(CookieJar, Json<TotpResp>)> {
    let s = pending(&st, &jar).await?;
    if st.limiter.blocked(&client.ip, Some(&s.username)) {
        return Err(ApiError::TooManyRequests);
    }
    let secret = s
        .totp_secret(&st)?
        .ok_or_else(|| ApiError::BadRequest("open TOTP setup first".into()))?;
    let mut ok = totp::check(&secret, &s.username, &req.code);
    let mut via_recovery = false;
    if !ok && s.stage == "pending_totp" && req.code.trim().len() > 6 {
        ok = use_recovery_code(&st, s.user_id, &req.code).await?;
        via_recovery = ok;
    }
    let audit_entry = |ok: bool, action: &'static str| Entry {
        user_id: Some(s.user_id),
        username: Some(&s.username),
        action,
        target: None,
        ip: &client.ip,
        ok,
        details: json!({ "method": s.method, "recovery_code": via_recovery }),
    };
    if !ok {
        st.limiter.fail(&client.ip, Some(&s.username));
        audit::record(&st, audit_entry(false, "auth.totp_failed")).await;
        return Err(ApiError::Forbidden("invalid_code", "wrong code".into()));
    }
    st.limiter.reset_user(&s.username);

    let mut recovery_codes = None;
    if s.stage == "totp_setup" {
        let codes: Vec<String> = (0..10)
            .map(|_| {
                use rand::Rng;
                let raw: String = rand::thread_rng()
                    .sample_iter(rand::distributions::Alphanumeric)
                    .take(10)
                    .map(|c| char::from(c).to_ascii_lowercase())
                    .collect();
                format!("{}-{}", &raw[..5], &raw[5..])
            })
            .collect();
        let mut tx = st.db.begin().await?;
        sqlx::query("UPDATE users SET totp_enabled = TRUE WHERE id = $1")
            .bind(s.user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM recovery_codes WHERE user_id = $1")
            .bind(s.user_id)
            .execute(&mut *tx)
            .await?;
        for c in &codes {
            let norm: String = c.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES ($1, $2)")
                .bind(s.user_id)
                .bind(sha256_hex(&norm))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        recovery_codes = Some(codes);
        audit::record(&st, audit_entry(true, "auth.totp_enabled")).await;
    }

    // Rotate the token on promotion so a pre-auth cookie never becomes a full session.
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(&s.token_hash)
        .execute(&st.db)
        .await?;
    let jar = create_session(&st, jar, &client, s.user_id, "active", &s.method).await?;
    audit::record(&st, audit_entry(true, "auth.login")).await;
    Ok((jar, Json(TotpResp { recovery_codes })))
}

pub async fn telegram(
    State(st): State<AppState>,
    client: ClientInfo,
    jar: CookieJar,
    Json(data): Json<Map<String, Value>>,
) -> ApiResult<(CookieJar, Json<Value>)> {
    let token = st
        .cfg
        .telegram_bot_token
        .as_deref()
        .ok_or(ApiError::NotFound)?;
    if st.limiter.blocked(&client.ip, None) {
        return Err(ApiError::TooManyRequests);
    }
    let tg = telegram::verify(token, &data, Utc::now().timestamp());
    let user: Option<(i64, String)> =
        match &tg {
            Some(tg) => sqlx::query_as(
                "SELECT u.id, u.username FROM user_identities i JOIN users u ON u.id = i.user_id
                  WHERE i.provider = 'telegram' AND i.subject = $1 AND NOT u.disabled",
            )
            .bind(tg.id.to_string())
            .fetch_optional(&st.db)
            .await?,
            None => None,
        };
    let Some((user_id, username)) = user else {
        st.limiter.fail(&client.ip, None);
        audit::record(
            &st,
            Entry {
                user_id: None,
                username: None,
                action: "auth.login_failed",
                target: tg.as_ref().map(|t| t.id.to_string()).as_deref(),
                ip: &client.ip,
                ok: false,
                details: json!({ "method": "telegram", "signature_valid": tg.is_some() }),
            },
        )
        .await;
        return Err(ApiError::Forbidden(
            "telegram_denied",
            "this Telegram account is not allowed".into(),
        ));
    };
    let jar = create_session(&st, jar, &client, user_id, "active", "telegram").await?;
    audit::record(&st, Entry {
        user_id: Some(user_id),
        username: Some(&username),
        action: "auth.login",
        target: None,
        ip: &client.ip,
        ok: true,
        details: json!({ "method": "telegram", "telegram_username": tg.and_then(|t| t.username) }),
    })
    .await;
    Ok((jar, Json(json!({ "ok": true }))))
}

pub async fn sudo(
    State(st): State<AppState>,
    user: AuthUser,
    client: ClientInfo,
    Json(req): Json<CodeReq>,
) -> ApiResult<Json<Value>> {
    if st.limiter.blocked(&client.ip, Some(&user.username)) {
        return Err(ApiError::TooManyRequests);
    }
    let secret = user
        .totp_secret(&st)?
        .filter(|_| user.totp_enabled)
        .ok_or_else(|| {
            ApiError::Forbidden(
                "totp_not_configured",
                "sign in with a password once to set up TOTP".into(),
            )
        })?;
    let ok = totp::check(&secret, &user.username, &req.code);
    audit::record(
        &st,
        Entry {
            user_id: Some(user.user_id),
            username: Some(&user.username),
            action: "auth.sudo",
            target: None,
            ip: &client.ip,
            ok,
            details: json!({}),
        },
    )
    .await;
    if !ok {
        st.limiter.fail(&client.ip, Some(&user.username));
        return Err(ApiError::Forbidden("invalid_code", "wrong code".into()));
    }
    let until: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE sessions SET sudo_until = now() + $2::interval WHERE token_hash = $1 RETURNING sudo_until",
    )
    .bind(&user.token_hash)
    .bind(format!("{} seconds", SUDO_TTL.num_seconds()))
    .fetch_one(&st.db)
    .await?;
    Ok(Json(json!({ "sudo_until": until })))
}

pub async fn logout(State(st): State<AppState>, jar: CookieJar) -> ApiResult<CookieJar> {
    let name = session_cookie_name(st.cfg.cookie_secure);
    if let Some(c) = jar.get(name) {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(sha256_hex(c.value()))
            .execute(&st.db)
            .await?;
    }
    Ok(jar.remove(session_cookie(&st, String::new())))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct SessionRow {
    method: String,
    ip: Option<String>,
    user_agent: Option<String>,
    created_at: DateTime<Utc>,
    last_seen: DateTime<Utc>,
    current: bool,
}

pub async fn sessions(
    State(st): State<AppState>,
    user: AuthUser,
) -> ApiResult<Json<Vec<SessionRow>>> {
    let rows = sqlx::query_as::<_, SessionRow>(
        "SELECT method, ip, user_agent, created_at, last_seen, token_hash = $2 AS current
           FROM sessions WHERE user_id = $1 AND stage = 'active' AND expires_at > now()
          ORDER BY last_seen DESC",
    )
    .bind(user.user_id)
    .bind(&user.token_hash)
    .fetch_all(&st.db)
    .await?;
    Ok(Json(rows))
}
