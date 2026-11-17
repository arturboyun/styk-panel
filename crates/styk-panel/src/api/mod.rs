mod actions;
mod alerts;
mod audit;
mod auth;
mod backups;
mod docker;
pub mod domains;
mod files;
mod jobs;
mod security;
mod system;

use axum::routing::{get, post};
use axum::{middleware, Router};

use crate::state::AppState;

pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/auth/config", get(auth::config))
        .route("/auth/me", get(auth::me))
        .route("/auth/login", post(auth::login))
        .route("/auth/totp/setup", get(auth::totp_setup))
        .route("/auth/totp", post(auth::totp_verify))
        .route("/auth/telegram", post(auth::telegram))
        .route("/auth/sudo", post(auth::sudo))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/sessions", get(auth::sessions))
        .route("/system/overview", get(system::overview))
        .route("/system/history", get(system::history))
        .route("/system/live", get(system::live))
        .route("/system/version", get(crate::update::version))
        .route("/system/update", post(crate::update::update))
        .route("/docker/projects", get(docker::projects))
        .route("/docker/df", get(docker::df))
        .route("/docker/containers/{id}", get(docker::inspect))
        .route("/docker/containers/{id}/logs", get(docker::logs))
        .route(
            "/docker/projects/{name}/action",
            post(actions::project_action),
        )
        .route(
            "/docker/projects/{name}/hook",
            post(actions::create_hook).delete(actions::delete_hook),
        )
        .route("/docker/hooks", get(actions::hooks))
        .route(
            "/docker/containers/{id}/action",
            post(actions::container_action),
        )
        .route("/docker/containers/{id}/exec", get(actions::exec))
        .route("/docker/prune", post(actions::prune))
        .route(
            "/docker/images/retention",
            get(actions::retention).post(actions::save_retention),
        )
        .route("/docker/images/retention/run", post(actions::run_retention))
        .route("/hooks/deploy/{name}", post(actions::deploy_hook))
        .route("/alerts", get(alerts::overview))
        .route("/alerts/rules/{key}", post(alerts::update_rule))
        .route("/alerts/rules/{key}/mute", post(alerts::mute_rule))
        .route("/alerts/events/{id}/ack", post(alerts::ack))
        .route("/alerts/events/{id}/mute", post(alerts::mute_event))
        .route("/alerts/settings", post(alerts::save_settings))
        .route("/alerts/telegram/test", post(alerts::telegram_test))
        .route("/alerts/summary", post(alerts::summary_now))
        .route("/uptime", get(alerts::uptime).post(alerts::add_check))
        .route("/uptime/{id}", axum::routing::delete(alerts::delete_check))
        .route("/uptime/{id}/toggle", post(alerts::toggle_check))
        .route("/uptime/{id}/history", get(alerts::uptime_history))
        .route("/audit", get(audit::list))
        .route("/jobs", get(jobs::list))
        .route("/backups/overview", get(backups::overview))
        .route(
            "/backups/restore-test/schedule",
            post(backups::restore_test_schedule),
        )
        .route("/system/timers", get(backups::timers))
        .route("/system/units/{unit}", get(backups::unit))
        .route("/system/units/{unit}/journal", get(backups::unit_journal))
        .route("/system/units/{unit}/run", post(backups::run_unit))
        .route("/databases", get(backups::databases))
        .route("/databases/{container}/dump", post(backups::dump))
        .route("/databases/{container}/restore", post(backups::restore))
        .route("/databases/{container}/query", post(backups::query))
        .route("/domains", get(domains::list).post(domains::add))
        .route("/domains/upstreams", get(domains::upstreams))
        .route("/domains/preview", post(domains::preview))
        .route(
            "/domains/caddyfile",
            get(domains::get_file).post(domains::save_file),
        )
        .route(
            "/domains/site/{domain}",
            axum::routing::delete(domains::remove),
        )
        .route("/files/roots", get(files::roots))
        .route("/files/list", get(files::list))
        .route("/files/read", get(files::read))
        .route("/files/download", get(files::download))
        .route("/files/write", post(files::write))
        .route("/files/upload", post(files::upload))
        .route("/files/mkdir", post(files::mkdir))
        .route("/files/rename", post(files::rename))
        .route("/files/trash", post(files::trash))
        .route("/files/history", get(files::history))
        .route("/files/version", get(files::version))
        .route("/security/overview", get(security::overview))
        .route("/security/ssh", get(security::ssh))
        .route("/security/firewall", get(security::firewall))
        .route(
            "/security/access",
            get(security::access).post(security::save_access),
        )
        .route("/security/updates", get(security::updates))
        .route("/security/updates/apply", post(security::apply_updates))
        .route("/jobs/{id}", get(jobs::get))
        .layer(middleware::from_fn_with_state(state, crate::auth::csrf))
}
