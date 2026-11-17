// sqlx row tuples read better inline than as type aliases.
#![allow(clippy::type_complexity)]

mod agent;
mod alerts;
mod api;
mod audit;
mod auth;
mod cache;
mod caddyfile;
mod certs;
mod cloudflare;
mod config;
mod crypto;
mod docker;
mod error;
mod images;
mod jobs;
mod metrics;
mod notify;
mod settings;
mod state;
mod update;
mod uptime;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use axum::http::{header, HeaderValue};
use axum::routing::get;
use axum::Router;
use clap::{Parser, Subcommand};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::state::{AppState, Inner};

#[derive(Parser)]
#[command(name = "styk-panel", about = "Hosting control panel")]
struct Cli {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true, global = true)]
    database_url: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // parsed once at startup
enum Cmd {
    /// Run the web server (applies migrations first).
    Serve(config::ServeConfig),
    /// Apply database migrations and exit.
    Migrate,
    #[command(subcommand)]
    User(UserCmd),
    /// Exit 0 if the local server answers /healthz (for the container healthcheck).
    Healthcheck {
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: String,
    },
    /// Call a styk-agent method, e.g. `agent '{"method":"ssh_activity","params":{"hours":24}}'`.
    Agent {
        request: String,
        #[arg(long, env = "STYK_AGENT_SOCKET", default_value = styk_proto::DEFAULT_SOCKET)]
        socket: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum UserCmd {
    /// Create a user; the password is read from STYK_PASSWORD or prompted.
    Create {
        username: String,
    },
    /// Set a new password (prompted).
    Passwd {
        username: String,
    },
    /// Drop TOTP so it is enrolled again on next password sign-in.
    ResetTotp {
        username: String,
    },
    /// Allow signing in as this user with Telegram Login.
    LinkTelegram {
        username: String,
        telegram_id: i64,
    },
    UnlinkTelegram {
        username: String,
        telegram_id: i64,
    },
    /// Block sign-in and end all sessions.
    Disable {
        username: String,
    },
    Enable {
        username: String,
    },
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    if let Cmd::Healthcheck { addr } = &cli.cmd {
        return healthcheck(addr).await;
    }
    if let Cmd::Agent { request, socket } = &cli.cmd {
        let method: styk_proto::Method =
            serde_json::from_str(request).context("parse the method JSON")?;
        let client = agent::AgentClient::new(socket.clone());
        let v: serde_json::Value = client
            .call_timeout(method, std::time::Duration::from_secs(120))
            .await?;
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let db_url = cli.database_url.context("DATABASE_URL is required")?;
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&db_url)
        .await
        .context("connect to Postgres")?;
    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .context("run migrations")?;

    match cli.cmd {
        Cmd::Migrate => Ok(()),
        Cmd::User(cmd) => user_cmd(&db, cmd).await,
        Cmd::Serve(cfg) => serve(db, cfg.normalize()).await,
        Cmd::Healthcheck { .. } | Cmd::Agent { .. } => unreachable!(),
    }
}

async fn healthcheck(addr: &str) -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect(addr),
    )
    .await??;
    s.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .await?;
    let mut buf = [0u8; 16];
    let n = s.read(&mut buf).await?;
    if !buf[..n].starts_with(b"HTTP/1.0 200") && !buf[..n].starts_with(b"HTTP/1.1 200") {
        bail!("unhealthy");
    }
    Ok(())
}

async fn serve(db: PgPool, cfg: config::ServeConfig) -> Result<()> {
    let docker = bollard::Docker::connect_with_local_defaults().context("connect to Docker")?;
    docker.ping().await.context("ping Docker")?;
    let state = AppState(Arc::new(Inner {
        cipher: crypto::Cipher::from_base64(&cfg.secret_key)?,
        agent: Arc::new(agent::AgentClient::new(cfg.agent_socket.clone())),
        metrics: Arc::default(),
        limiter: Default::default(),
        jobs: Default::default(),
        cache: Default::default(),
        caddy_lock: Default::default(),
        alerts: Default::default(),
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent(concat!("styk-panel/", env!("CARGO_PKG_VERSION")))
            .build()?,
        db: db.clone(),
        docker,
        cfg: cfg.clone(),
    }));

    sqlx::query("DELETE FROM sessions WHERE expires_at < now()")
        .execute(&db)
        .await?;
    metrics::spawn(state.clone());
    jobs::resume(&state).await;
    uptime::spawn(state.clone());
    alerts::spawn(state.clone());
    notify::spawn(state.clone());
    images::spawn(state.clone());
    update::spawn(state.clone());

    let static_files = ServeDir::new(&cfg.static_dir)
        .append_index_html_on_directories(true)
        .precompressed_br()
        .precompressed_gzip()
        .not_found_service(ServeFile::new(cfg.static_dir.join("404.html")));

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api", api::router(state.clone()))
        .fallback_service(static_files)
        .layer(axum::middleware::map_response(cache_control))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self' 'unsafe-inline' https://telegram.org; \
                 frame-src https://oauth.telegram.org; img-src 'self' data: blob: https://t.me https://*.telegram.org; \
                 style-src 'self' 'unsafe-inline'; font-src 'self' data:; connect-src 'self'; \
                 worker-src 'self' blob:; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("same-origin")))
        .layer(SetResponseHeaderLayer::overriding(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(cfg.listen).await?;
    tracing::info!("styk-panel listening on {}", cfg.listen);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

/// Next puts a content hash in every `/_next/static` name, so those never change; pages
/// and the API must always be revalidated. API handlers that set their own header keep it.
async fn cache_control(
    uri: axum::http::Uri,
    mut res: axum::response::Response,
) -> axum::response::Response {
    let value = if uri.path().starts_with("/_next/static/") && res.status().is_success() {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    res.headers_mut()
        .entry(header::CACHE_CONTROL)
        .or_insert(HeaderValue::from_static(value));
    res
}

fn read_new_password() -> Result<String> {
    if let Ok(p) = std::env::var("STYK_PASSWORD") {
        return Ok(p);
    }
    let p = rpassword::prompt_password("Password: ")?;
    if p != rpassword::prompt_password("Repeat:   ")? {
        bail!("passwords do not match");
    }
    Ok(p)
}

async fn user_id(db: &PgPool, username: &str) -> Result<i64> {
    sqlx::query_scalar("SELECT id FROM users WHERE lower(username) = lower($1)")
        .bind(username)
        .fetch_optional(db)
        .await?
        .with_context(|| format!("no user {username}"))
}

async fn user_cmd(db: &PgPool, cmd: UserCmd) -> Result<()> {
    match cmd {
        UserCmd::Create { username } => {
            let pw = read_new_password()?;
            if pw.len() < 12 {
                bail!("use at least 12 characters");
            }
            sqlx::query("INSERT INTO users (username, password_hash) VALUES ($1, $2)")
                .bind(&username)
                .bind(auth::password::hash(&pw)?)
                .execute(db)
                .await?;
            println!("created {username}; TOTP is enrolled on first sign-in");
        }
        UserCmd::Passwd { username } => {
            let id = user_id(db, &username).await?;
            let pw = read_new_password()?;
            if pw.len() < 12 {
                bail!("use at least 12 characters");
            }
            sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
                .bind(auth::password::hash(&pw)?)
                .bind(id)
                .execute(db)
                .await?;
            sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                .bind(id)
                .execute(db)
                .await?;
            println!("password changed, sessions ended");
        }
        UserCmd::ResetTotp { username } => {
            let id = user_id(db, &username).await?;
            sqlx::query(
                "UPDATE users SET totp_enabled = FALSE, totp_secret_enc = NULL WHERE id = $1",
            )
            .bind(id)
            .execute(db)
            .await?;
            sqlx::query("DELETE FROM recovery_codes WHERE user_id = $1")
                .bind(id)
                .execute(db)
                .await?;
            sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                .bind(id)
                .execute(db)
                .await?;
            println!("TOTP reset, sessions ended");
        }
        UserCmd::LinkTelegram {
            username,
            telegram_id,
        } => {
            let id = user_id(db, &username).await?;
            sqlx::query("INSERT INTO user_identities (user_id, provider, subject) VALUES ($1, 'telegram', $2)")
                .bind(id)
                .bind(telegram_id.to_string())
                .execute(db)
                .await?;
            println!("linked Telegram {telegram_id} to {username}");
        }
        UserCmd::UnlinkTelegram {
            username,
            telegram_id,
        } => {
            let id = user_id(db, &username).await?;
            sqlx::query("DELETE FROM user_identities WHERE user_id = $1 AND provider = 'telegram' AND subject = $2")
                .bind(id)
                .bind(telegram_id.to_string())
                .execute(db)
                .await?;
            println!("unlinked");
        }
        UserCmd::Disable { username } => {
            let id = user_id(db, &username).await?;
            sqlx::query("UPDATE users SET disabled = TRUE WHERE id = $1")
                .bind(id)
                .execute(db)
                .await?;
            sqlx::query("DELETE FROM sessions WHERE user_id = $1")
                .bind(id)
                .execute(db)
                .await?;
            println!("disabled {username}");
        }
        UserCmd::Enable { username } => {
            let id = user_id(db, &username).await?;
            sqlx::query("UPDATE users SET disabled = FALSE WHERE id = $1")
                .bind(id)
                .execute(db)
                .await?;
            println!("enabled {username}");
        }
        UserCmd::List => {
            let rows: Vec<(String, bool, bool, Option<String>)> = sqlx::query_as(
                "SELECT u.username, u.totp_enabled, u.disabled, string_agg(i.subject, ',')
                   FROM users u LEFT JOIN user_identities i ON i.user_id = u.id AND i.provider = 'telegram'
                  GROUP BY u.id ORDER BY u.username",
            )
            .fetch_all(db)
            .await?;
            for (u, totp, disabled, tg) in rows {
                println!(
                    "{u}\ttotp={totp}\tdisabled={disabled}\ttelegram={}",
                    tg.unwrap_or_default()
                );
            }
        }
    }
    Ok(())
}
