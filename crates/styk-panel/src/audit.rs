use serde_json::Value;

use crate::state::AppState;

pub struct Entry<'a> {
    pub user_id: Option<i64>,
    pub username: Option<&'a str>,
    pub action: &'a str,
    pub target: Option<&'a str>,
    pub ip: &'a str,
    pub ok: bool,
    pub details: Value,
}

/// Audit failures are logged but never fail the request that caused them.
pub async fn record(state: &AppState, e: Entry<'_>) {
    let res = sqlx::query(
        "INSERT INTO audit_log (user_id, username, action, target, ip, ok, details) VALUES ($1,$2,$3,$4,$5,$6,$7)",
    )
    .bind(e.user_id)
    .bind(e.username)
    .bind(e.action)
    .bind(e.target)
    .bind(e.ip)
    .bind(e.ok)
    .bind(&e.details)
    .execute(&state.db)
    .await;
    if let Err(err) = res {
        tracing::error!(action = e.action, "audit insert failed: {err}");
    }
}

/// Shorthand for actions taken by a signed-in user.
pub async fn user_action(
    state: &AppState,
    user: &crate::auth::AuthUser,
    client: &crate::auth::ClientInfo,
    action: &str,
    target: &str,
    ok: bool,
    details: Value,
) {
    record(
        state,
        Entry {
            user_id: Some(user.user_id),
            username: Some(&user.username),
            action,
            target: Some(target),
            ip: &client.ip,
            ok,
            details,
        },
    )
    .await
}
