//! Telegram notifications through the same bot as Telegram Login.
//!
//! Outgoing: `sendMessage` to the chat configured on the Alerts page. Incoming:
//! long-polling `getUpdates`; inline buttons (ack / mute) and commands are accepted only
//! from Telegram accounts linked to a panel user. Without TELEGRAM_BOT_TOKEN everything
//! here is a no-op.

use std::time::Duration;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::settings;
use crate::state::AppState;

pub const CHAT_KEY: &str = "telegram.chat_id";
pub const LAST_START_KEY: &str = "telegram.last_start";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastStart {
    pub chat_id: i64,
    pub from_id: i64,
    pub name: String,
    pub linked: bool,
    pub ts: i64,
}

fn api(st: &AppState, method: &str) -> Option<String> {
    let token = st.cfg.telegram_bot_token.as_deref()?;
    Some(format!("https://api.telegram.org/bot{token}/{method}"))
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn call(st: &AppState, method: &str, body: Value, timeout: Duration) -> Result<Value> {
    let Some(url) = api(st, method) else {
        bail!("TELEGRAM_BOT_TOKEN is not set")
    };
    let v: Value = st
        .http
        .post(url)
        .timeout(timeout)
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    if v["ok"] != true {
        bail!(
            "telegram {method}: {}",
            v["description"].as_str().unwrap_or("error")
        );
    }
    Ok(v["result"].clone())
}

pub async fn chat_id(st: &AppState) -> Option<i64> {
    settings::get(st, CHAT_KEY).await
}

/// Sends an HTML message; `buttons` are rows of inline keyboard buttons.
pub async fn send_to(st: &AppState, chat: i64, html: &str, buttons: Value) -> Result<()> {
    let mut body = json!({ "chat_id": chat, "text": html, "parse_mode": "HTML", "disable_web_page_preview": true });
    if buttons.as_array().is_some_and(|a| !a.is_empty()) {
        body["reply_markup"] = json!({ "inline_keyboard": buttons });
    }
    call(st, "sendMessage", body, Duration::from_secs(15)).await?;
    Ok(())
}

pub async fn send(st: &AppState, html: &str, buttons: Value) -> Result<bool> {
    let Some(chat) = chat_id(st).await else {
        return Ok(false);
    };
    if st.cfg.telegram_bot_token.is_none() {
        return Ok(false);
    }
    send_to(st, chat, html, buttons).await?;
    Ok(true)
}

async fn linked_user(st: &AppState, telegram_id: i64) -> Option<String> {
    sqlx::query_scalar(
        "SELECT u.username FROM user_identities i JOIN users u ON u.id = i.user_id
          WHERE i.provider = 'telegram' AND i.subject = $1 AND NOT u.disabled",
    )
    .bind(telegram_id.to_string())
    .fetch_optional(&st.db)
    .await
    .ok()
    .flatten()
}

pub fn spawn(st: AppState) {
    if st.cfg.telegram_bot_token.is_none() {
        return;
    }
    tokio::spawn(async move {
        let mut offset: i64 = 0;
        let mut warned = false;
        loop {
            let res = call(
                &st,
                "getUpdates",
                json!({ "offset": offset, "timeout": 50, "allowed_updates": ["message", "callback_query"] }),
                Duration::from_secs(65),
            )
            .await;
            match res {
                Ok(updates) => {
                    warned = false;
                    for u in updates.as_array().into_iter().flatten() {
                        offset = offset.max(u["update_id"].as_i64().unwrap_or(0) + 1);
                        if let Err(e) = handle(&st, u).await {
                            tracing::warn!("telegram update: {e:#}");
                        }
                    }
                }
                Err(e) => {
                    if !warned {
                        tracing::warn!("telegram getUpdates: {e:#}");
                        warned = true;
                    }
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            }
        }
    });
}

async fn handle(st: &AppState, u: &Value) -> Result<()> {
    if let Some(cb) = u.get("callback_query") {
        let from = cb["from"]["id"].as_i64().unwrap_or(0);
        let id = cb["id"].as_str().unwrap_or("").to_string();
        let answer = match linked_user(st, from).await {
            None => "Этот Telegram-аккаунт не привязан к панели".to_string(),
            Some(user) => {
                let data = cb["data"].as_str().unwrap_or("");
                crate::alerts::telegram_action(st, &user, data).await
            }
        };
        call(
            st,
            "answerCallbackQuery",
            json!({ "callback_query_id": id, "text": answer }),
            Duration::from_secs(10),
        )
        .await?;
        return Ok(());
    }
    let Some(msg) = u.get("message") else {
        return Ok(());
    };
    let text = msg["text"].as_str().unwrap_or("");
    let chat = msg["chat"]["id"].as_i64().unwrap_or(0);
    let from = msg["from"]["id"].as_i64().unwrap_or(0);
    let user = linked_user(st, from).await;
    if text.starts_with("/start") {
        let name = msg["chat"]["title"]
            .as_str()
            .or(msg["from"]["username"].as_str())
            .or(msg["from"]["first_name"].as_str())
            .unwrap_or("")
            .to_string();
        let last = LastStart {
            chat_id: chat,
            from_id: from,
            name,
            linked: user.is_some(),
            ts: chrono::Utc::now().timestamp(),
        };
        settings::set(st, LAST_START_KEY, &last).await?;
        let reply = if user.is_some() {
            "Готово. Откройте «Алерты» в панели и нажмите «Найти чат», чтобы получать уведомления сюда."
        } else {
            "Этот аккаунт не привязан к Styk Panel. Привяжите его: <code>styk-panel user link-telegram &lt;login&gt; ID</code>"
        };
        send_to(st, chat, &reply.replace("ID", &from.to_string()), json!([])).await?;
    } else if text.starts_with("/status") && user.is_some() {
        let html = crate::alerts::summary(st).await;
        send_to(st, chat, &html, json!([])).await?;
    }
    Ok(())
}
