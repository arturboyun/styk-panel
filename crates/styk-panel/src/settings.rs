//! Small key/value store for runtime settings (alert chat id, quiet hours, …).

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::state::AppState;

pub async fn get<T: DeserializeOwned>(state: &AppState, key: &str) -> Option<T> {
    let v: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = $1")
            .bind(key)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    v.and_then(|v| serde_json::from_value(v).ok())
}

pub async fn set<T: Serialize>(state: &AppState, key: &str, value: &T) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES ($1, $2)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(key)
    .bind(serde_json::to_value(value).unwrap_or_default())
    .execute(&state.db)
    .await?;
    Ok(())
}
