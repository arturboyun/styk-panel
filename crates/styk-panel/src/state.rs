use std::sync::Arc;

use sqlx::PgPool;

use crate::agent::AgentClient;
use crate::auth::limiter::LoginLimiter;
use crate::cache::Cache;
use crate::config::ServeConfig;
use crate::crypto::Cipher;
use crate::jobs::Jobs;
use crate::metrics::Metrics;

#[derive(Clone)]
pub struct AppState(pub Arc<Inner>);

pub struct Inner {
    pub cfg: ServeConfig,
    pub db: PgPool,
    pub cipher: Cipher,
    pub docker: bollard::Docker,
    pub agent: Arc<AgentClient>,
    pub metrics: Arc<Metrics>,
    pub limiter: LoginLimiter,
    pub jobs: Jobs,
    pub cache: Cache,
    /// Serialises Caddyfile edits (read → validate → write → reload).
    pub caddy_lock: tokio::sync::Mutex<()>,
    pub http: reqwest::Client,
    pub alerts: crate::alerts::Runtime,
}

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}
