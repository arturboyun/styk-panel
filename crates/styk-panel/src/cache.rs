//! Tiny TTL cache for expensive read-only calls (agent, Docker, journal scans, apt, certs).
//!
//! Single-flight: concurrent misses on one key wait for the first caller's result instead of
//! running the same slow call in parallel. Errors are not cached.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

type Slot = Arc<tokio::sync::Mutex<Option<(Instant, Value)>>>;

/// Entries untouched this long are dropped on the next insert.
const MAX_AGE: Duration = Duration::from_secs(3 * 3600);

#[derive(Default)]
pub struct Cache {
    map: Mutex<HashMap<String, Slot>>,
}

impl Cache {
    pub async fn get_or<F, Fut, E>(&self, key: &str, ttl: Duration, f: F) -> Result<Value, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Value, E>>,
    {
        let slot = {
            let mut map = self.map.lock().unwrap();
            if let Some(s) = map.get(key) {
                s.clone()
            } else {
                map.retain(|_, s| match s.try_lock() {
                    Ok(v) => v.as_ref().is_some_and(|(t, _)| t.elapsed() < MAX_AGE),
                    Err(_) => true,
                });
                let s = Slot::default();
                map.insert(key.to_string(), s.clone());
                s
            }
        };
        let mut guard = slot.lock().await;
        if let Some((t, v)) = guard.as_ref() {
            if t.elapsed() < ttl {
                return Ok(v.clone());
            }
        }
        let v = f().await?;
        *guard = Some((Instant::now(), v.clone()));
        Ok(v)
    }

    pub fn invalidate(&self, prefix: &str) {
        self.map
            .lock()
            .unwrap()
            .retain(|k, _| !k.starts_with(prefix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn single_flight() {
        let cache = Arc::new(Cache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let (cache, calls) = (cache.clone(), calls.clone());
                tokio::spawn(async move {
                    cache
                        .get_or("k", Duration::from_secs(60), || async {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            Ok::<_, ()>(Value::from(1))
                        })
                        .await
                })
            })
            .collect();
        for t in tasks {
            assert_eq!(t.await.unwrap(), Ok(Value::from(1)));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        let cache = Cache::default();
        let r: Result<Value, &str> = cache
            .get_or("k", Duration::from_secs(60), || async { Err("x") })
            .await;
        assert!(r.is_err());
        let r: Result<Value, &str> = cache
            .get_or("k", Duration::from_secs(60), || async {
                Ok(Value::from(2))
            })
            .await;
        assert_eq!(r, Ok(Value::from(2)));
    }
}
