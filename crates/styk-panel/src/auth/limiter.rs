//! In-memory brute-force limiter for sign-in endpoints.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_PER_USER: usize = 5;
const MAX_PER_IP: usize = 20;

#[derive(Default)]
pub struct LoginLimiter(Mutex<HashMap<String, Vec<Instant>>>);

impl LoginLimiter {
    fn count(&self, key: &str) -> usize {
        let mut m = self.0.lock().unwrap();
        let now = Instant::now();
        m.retain(|_, v| {
            v.retain(|t| now.duration_since(*t) < WINDOW);
            !v.is_empty()
        });
        m.get(key).map_or(0, Vec::len)
    }

    pub fn blocked(&self, ip: &str, user: Option<&str>) -> bool {
        self.count(&format!("ip:{ip}")) >= MAX_PER_IP
            || user
                .is_some_and(|u| self.count(&format!("user:{}", u.to_lowercase())) >= MAX_PER_USER)
    }

    pub fn fail(&self, ip: &str, user: Option<&str>) {
        let mut m = self.0.lock().unwrap();
        m.entry(format!("ip:{ip}"))
            .or_default()
            .push(Instant::now());
        if let Some(u) = user {
            m.entry(format!("user:{}", u.to_lowercase()))
                .or_default()
                .push(Instant::now());
        }
    }

    pub fn reset_user(&self, user: &str) {
        self.0
            .lock()
            .unwrap()
            .remove(&format!("user:{}", user.to_lowercase()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_five_user_failures() {
        let l = LoginLimiter::default();
        for _ in 0..MAX_PER_USER {
            assert!(!l.blocked("1.1.1.1", Some("Admin")));
            l.fail("1.1.1.1", Some("admin"));
        }
        assert!(l.blocked("2.2.2.2", Some("ADMIN")));
        l.reset_user("admin");
        assert!(!l.blocked("2.2.2.2", Some("admin")));
    }
}
