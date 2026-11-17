use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{ArgAction, Args};

#[derive(Args, Debug, Clone)]
pub struct ServeConfig {
    #[arg(long, env = "STYK_LISTEN", default_value = "0.0.0.0:8080")]
    pub listen: SocketAddr,
    /// base64 of 32 random bytes, encrypts TOTP secrets at rest.
    #[arg(long, env = "STYK_SECRET_KEY", hide_env_values = true)]
    pub secret_key: String,
    #[arg(long, env = "STYK_AGENT_SOCKET", default_value = styk_proto::DEFAULT_SOCKET)]
    pub agent_socket: PathBuf,
    #[arg(long, env = "STYK_STATIC_DIR", default_value = "./frontend/out")]
    pub static_dir: PathBuf,
    /// Disable only for plain-http local development.
    #[arg(long, env = "STYK_COOKIE_SECURE", default_value_t = true, action = ArgAction::Set)]
    pub cookie_secure: bool,
    /// Take the client IP from CF-Connecting-IP / X-Forwarded-For. Enable only behind Caddy.
    #[arg(long, env = "STYK_TRUST_PROXY", default_value_t = false, action = ArgAction::Set)]
    pub trust_proxy: bool,
    #[arg(long, env = "TELEGRAM_BOT_TOKEN", hide_env_values = true)]
    pub telegram_bot_token: Option<String>,
    #[arg(long, env = "TELEGRAM_BOT_USERNAME")]
    pub telegram_bot_username: Option<String>,
    /// Compose projects the panel refuses to stop (itself and the shared proxy).
    #[arg(
        long,
        env = "STYK_PROTECTED_PROJECTS",
        value_delimiter = ',',
        default_value = "styk-panel"
    )]
    pub protected_projects: Vec<String>,
    /// Roots shown in the file manager (the agent enforces its own list).
    #[arg(
        long,
        env = "STYK_FS_ROOTS",
        value_delimiter = ',',
        default_value = "/opt,/srv,/var/backups"
    )]
    pub fs_roots: Vec<String>,
    /// The Caddyfile (host path, the agent writes it in place) and its container. The default
    /// is the Caddy that `install.sh` sets up; it mounts the whole caddy/ directory, so the
    /// file is never pinned by inode.
    #[arg(
        long,
        env = "STYK_CADDYFILE",
        default_value = "/opt/styk-panel/caddy/Caddyfile"
    )]
    pub caddyfile: String,
    #[arg(
        long,
        env = "STYK_CADDY_CONTAINER",
        default_value = "styk-panel-caddy-1"
    )]
    pub caddy_container: String,
    /// Path of the Caddyfile inside the Caddy container.
    #[arg(
        long,
        env = "STYK_CADDY_CONFIG",
        default_value = "/etc/caddy/Caddyfile"
    )]
    pub caddy_config: String,
    /// Optional: lets the domain wizard create proxied A records (needs Zone:DNS:Edit).
    #[arg(long, env = "CLOUDFLARE_API_TOKEN", hide_env_values = true)]
    pub cloudflare_api_token: Option<String>,
    /// Address for new DNS records.
    #[arg(long, env = "STYK_PUBLIC_IP")]
    pub public_ip: Option<String>,
    /// Dead man's switch: pinged every minute while the panel is healthy (GlitchTip/Healthchecks URL).
    #[arg(long, env = "STYK_HEARTBEAT_URL", hide_env_values = true)]
    pub heartbeat_url: Option<String>,
    /// Public URL of the panel (`https://panel.example.com`), used in notification links.
    #[arg(long, env = "STYK_PUBLIC_URL", default_value = "")]
    pub public_url: String,
    /// GitHub repository whose releases the panel offers as updates; empty turns it off.
    #[arg(
        long,
        env = "STYK_UPDATE_REPO",
        default_value = "arturboyun/styk-panel"
    )]
    pub update_repo: String,
    /// The panel's Postgres container, dumped daily to /var/backups/styk-panel (14 days
    /// kept); empty turns it off, e.g. when another backup already covers it.
    #[arg(long, env = "STYK_DB_CONTAINER", default_value = "styk-panel-db-1")]
    pub db_container: String,
}

impl ServeConfig {
    /// `KEY=` in an env file means "not set", not an empty value.
    pub fn normalize(mut self) -> Self {
        for v in [
            &mut self.telegram_bot_token,
            &mut self.telegram_bot_username,
            &mut self.cloudflare_api_token,
            &mut self.public_ip,
            &mut self.heartbeat_url,
        ] {
            *v = v
                .take()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        if let Some(u) = &mut self.telegram_bot_username {
            *u = u.trim_start_matches('@').to_string();
        }
        self
    }
}
