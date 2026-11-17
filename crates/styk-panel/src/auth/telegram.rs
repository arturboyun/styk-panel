//! Telegram Login Widget verification:
//! https://core.telegram.org/widgets/login#checking-authorization

use hmac::{Hmac, Mac};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const MAX_AGE_SECS: i64 = 600;

#[derive(Debug, PartialEq)]
pub struct TelegramUser {
    pub id: i64,
    pub username: Option<String>,
}

pub fn verify(bot_token: &str, data: &Map<String, Value>, now: i64) -> Option<TelegramUser> {
    let hash = data.get("hash")?.as_str()?;
    let mut pairs: Vec<String> = data
        .iter()
        .filter(|(k, v)| k.as_str() != "hash" && !v.is_null())
        .map(|(k, v)| match v {
            Value::String(s) => format!("{k}={s}"),
            v => format!("{k}={v}"),
        })
        .collect();
    pairs.sort();
    let check = pairs.join("\n");

    let key = Sha256::digest(bot_token.as_bytes());
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).ok()?;
    mac.update(check.as_bytes());
    mac.verify_slice(&hex::decode(hash).ok()?).ok()?;

    let auth_date = data
        .get("auth_date")
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))?;
    if now - auth_date > MAX_AGE_SECS || auth_date - now > 60 {
        return None;
    }
    Some(TelegramUser {
        id: data
            .get("id")
            .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))?,
        username: data
            .get("username")
            .and_then(|v| v.as_str())
            .map(Into::into),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(token: &str, fields: &[(&str, Value)]) -> Map<String, Value> {
        let mut m: Map<String, Value> = fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let mut pairs: Vec<String> = m
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => format!("{k}={s}"),
                v => format!("{k}={v}"),
            })
            .collect();
        pairs.sort();
        let mut mac = Hmac::<Sha256>::new_from_slice(&Sha256::digest(token.as_bytes())).unwrap();
        mac.update(pairs.join("\n").as_bytes());
        m.insert(
            "hash".into(),
            Value::String(hex::encode(mac.finalize().into_bytes())),
        );
        m
    }

    #[test]
    fn accepts_valid_and_rejects_tampered() {
        let now = 1_700_000_000;
        let mut d = sign(
            "123:abc",
            &[
                ("id", 42.into()),
                ("username", "alice".into()),
                ("auth_date", now.into()),
            ],
        );
        assert_eq!(
            verify("123:abc", &d, now + 5),
            Some(TelegramUser {
                id: 42,
                username: Some("alice".into())
            })
        );
        assert_eq!(verify("other", &d, now), None);
        assert_eq!(verify("123:abc", &d, now + MAX_AGE_SECS + 1), None);
        d.insert("id".into(), 43.into());
        assert_eq!(verify("123:abc", &d, now), None);
    }
}
