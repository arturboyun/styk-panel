//! Image retention: keep the newest N tags per repository, remove the rest.
//!
//! Never removed: images used by any container (running or stopped) and tags from the
//! protected list (`rollback`, `latest` by default). Removal is by tag without `force`,
//! so Docker itself refuses to delete anything a container still needs.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bollard::container::ListContainersOptions;
use bollard::image::{ListImagesOptions, RemoveImageOptions};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::audit;
use crate::error::ApiResult;
use crate::settings;
use crate::state::AppState;

const POLICY_KEY: &str = "images.retention";
const LAST_KEY: &str = "images.retention.last";
/// Daily run time (UTC), after backup-all.
const RUN_AT: (u32, u32) = (4, 30);

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Policy {
    pub enabled: bool,
    pub keep: usize,
    pub protected_tags: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: false,
            keep: 3,
            protected_tags: vec!["rollback".into(), "latest".into()],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LastRun {
    pub ts: i64,
    pub trigger: String,
    pub removed: usize,
    pub freed: i64,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Image {
    pub id: String,
    pub tags: Vec<String>,
    pub created: i64,
    pub size: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Ref {
    /// `repo:tag`, or the image id for untagged images.
    pub reference: String,
    pub id: String,
    pub created: i64,
    pub size: i64,
    /// Why it stays: "in_use", "protected", "newest"; empty for removals.
    pub reason: String,
}

#[derive(Serialize, Debug, Default)]
pub struct RepoPlan {
    pub repo: String,
    pub keep: Vec<Ref>,
    pub remove: Vec<Ref>,
}

#[derive(Serialize, Debug, Default)]
pub struct Plan {
    pub repos: Vec<RepoPlan>,
    pub remove_count: usize,
    /// Upper bound: images whose every tag goes away (shared layers count once per image).
    pub reclaim: i64,
}

fn split_ref(r: &str) -> (&str, &str) {
    // The tag is after the last ':' that comes after the last '/' (registry ports have ':').
    let slash = r.rfind('/').map_or(0, |i| i + 1);
    match r[slash..].rfind(':') {
        Some(i) => (&r[..slash + i], &r[slash + i + 1..]),
        None => (r, "latest"),
    }
}

pub fn plan(images: &[Image], used: &HashSet<String>, policy: &Policy) -> Plan {
    let mut by_repo: HashMap<String, Vec<(Ref, bool)>> = HashMap::new();
    for img in images {
        let in_use = used.contains(&img.id);
        let tags: Vec<&String> = img.tags.iter().filter(|t| *t != "<none>:<none>").collect();
        if tags.is_empty() {
            by_repo.entry("<none>".into()).or_default().push((
                Ref {
                    reference: img.id.clone(),
                    id: img.id.clone(),
                    created: img.created,
                    size: img.size,
                    reason: String::new(),
                },
                in_use,
            ));
        }
        for t in tags {
            by_repo
                .entry(split_ref(t).0.to_string())
                .or_default()
                .push((
                    Ref {
                        reference: t.clone(),
                        id: img.id.clone(),
                        created: img.created,
                        size: img.size,
                        reason: String::new(),
                    },
                    in_use,
                ));
        }
    }

    let mut out = Plan::default();
    let mut removed_tags: HashMap<&str, usize> = HashMap::new();
    let mut repos: Vec<_> = by_repo.into_iter().collect();
    repos.sort_by(|a, b| a.0.cmp(&b.0));
    for (repo, mut refs) in repos {
        refs.sort_by(|a, b| {
            b.0.created
                .cmp(&a.0.created)
                .then(a.0.reference.cmp(&b.0.reference))
        });
        let mut rp = RepoPlan {
            repo: repo.clone(),
            ..Default::default()
        };
        let mut kept = 0;
        for (mut r, in_use) in refs {
            let tag = split_ref(&r.reference).1;
            let reason = if in_use {
                kept += 1;
                "in_use"
            } else if repo != "<none>" && policy.protected_tags.iter().any(|p| p == tag) {
                "protected"
            } else if repo != "<none>" && kept < policy.keep {
                kept += 1;
                "newest"
            } else {
                ""
            };
            r.reason = reason.into();
            if reason.is_empty() {
                rp.remove.push(r);
            } else {
                rp.keep.push(r);
            }
        }
        out.remove_count += rp.remove.len();
        out.repos.push(rp);
    }
    for rp in &out.repos {
        for r in &rp.remove {
            *removed_tags.entry(r.id.as_str()).or_default() += 1;
        }
    }
    for img in images {
        let tags = img
            .tags
            .iter()
            .filter(|t| *t != "<none>:<none>")
            .count()
            .max(1);
        if removed_tags.get(img.id.as_str()) == Some(&tags) {
            out.reclaim += img.size;
        }
    }
    out
}

pub async fn policy(st: &AppState) -> Policy {
    settings::get(st, POLICY_KEY).await.unwrap_or_default()
}

pub async fn last_run(st: &AppState) -> Option<LastRun> {
    settings::get(st, LAST_KEY).await
}

pub async fn save_policy(st: &AppState, p: &Policy) -> sqlx::Result<()> {
    settings::set(st, POLICY_KEY, p).await
}

pub async fn current_plan(st: &AppState, policy: &Policy) -> ApiResult<Plan> {
    let images = st
        .docker
        .list_images(Some(ListImagesOptions::<String> {
            all: false,
            ..Default::default()
        }))
        .await?
        .into_iter()
        .map(|i| Image {
            id: i.id,
            tags: i.repo_tags,
            created: i.created,
            size: i.size,
        })
        .collect::<Vec<_>>();
    let used = st
        .docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        }))
        .await?
        .into_iter()
        .filter_map(|c| c.image_id)
        .collect::<HashSet<_>>();
    Ok(plan(&images, &used, policy))
}

/// Applies the policy now. `actor` is `None` for the daily timer.
pub async fn run(
    st: &AppState,
    policy: &Policy,
    trigger: &str,
    actor: Option<(&crate::auth::AuthUser, &crate::auth::ClientInfo)>,
) -> ApiResult<LastRun> {
    let plan = current_plan(st, policy).await?;
    let mut removed = 0;
    let mut errors = Vec::new();
    let mut gone: HashSet<String> = HashSet::new();
    for r in plan.repos.iter().flat_map(|rp| &rp.remove) {
        let opts = RemoveImageOptions {
            force: false,
            noprune: false,
        };
        match st.docker.remove_image(&r.reference, Some(opts), None).await {
            Ok(items) => {
                removed += 1;
                gone.extend(items.into_iter().filter_map(|i| i.deleted));
            }
            Err(e) => errors.push(format!("{}: {e}", r.reference)),
        }
    }
    let freed = plan
        .repos
        .iter()
        .flat_map(|rp| &rp.remove)
        .filter(|r| gone.contains(&r.id))
        .map(|r| (r.id.as_str(), r.size))
        .collect::<HashMap<_, _>>()
        .values()
        .sum();
    let last = LastRun {
        ts: Utc::now().timestamp(),
        trigger: trigger.into(),
        removed,
        freed,
        errors,
    };
    let _ = settings::set(st, LAST_KEY, &last).await;
    let details = json!({ "keep": policy.keep, "removed": removed, "freed": freed, "errors": last.errors.len() });
    let ok = last.errors.is_empty();
    match actor {
        Some((u, c)) => audit::user_action(st, u, c, "images.retention", "host", ok, details).await,
        None => {
            audit::record(
                st,
                audit::Entry {
                    user_id: None,
                    username: None,
                    action: "images.retention",
                    target: Some("host"),
                    ip: "system",
                    ok,
                    details,
                },
            )
            .await
        }
    }
    Ok(last)
}

/// Daily run at RUN_AT UTC when the policy is enabled.
pub fn spawn(st: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            let p = policy(&st).await;
            if !p.enabled {
                continue;
            }
            let now = Utc::now();
            let today = now.date_naive().and_hms_opt(RUN_AT.0, RUN_AT.1, 0).unwrap();
            let due = Utc.from_utc_datetime(&today).timestamp();
            let last = last_run(&st).await.map_or(0, |l| l.ts);
            if now.timestamp() >= due && last < due {
                match run(&st, &p, "timer", None).await {
                    Ok(l) => {
                        tracing::info!(removed = l.removed, freed = l.freed, "image retention")
                    }
                    Err(e) => tracing::warn!("image retention: {e:?}"),
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(id: &str, tags: &[&str], created: i64) -> Image {
        Image {
            id: id.into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            created,
            size: 100,
        }
    }

    #[test]
    fn splits_registry_ports() {
        assert_eq!(
            split_ref("reg.io:5000/ns/app:sha-1"),
            ("reg.io:5000/ns/app", "sha-1")
        );
        assert_eq!(split_ref("styk-panel:latest"), ("styk-panel", "latest"));
        assert_eq!(split_ref("reg.io:5000/app"), ("reg.io:5000/app", "latest"));
    }

    #[test]
    fn keeps_newest_used_and_protected() {
        let images = vec![
            img("a5", &["r/app:sha-5"], 5),
            img("a4", &["r/app:sha-4"], 4),
            img("a3", &["r/app:sha-3"], 3),
            img("a2", &["r/app:sha-2", "r/app:rollback"], 2),
            img("a1", &["r/app:sha-1"], 1),
            img("old", &["r/app:sha-0"], 0),
            img("dang", &["<none>:<none>"], 9),
            img("solo", &["other:latest"], 1),
        ];
        let used: HashSet<String> = ["old".to_string()].into();
        let p = plan(
            &images,
            &used,
            &Policy {
                keep: 2,
                ..Policy::default()
            },
        );
        let app = p.repos.iter().find(|r| r.repo == "r/app").unwrap();
        let keep: Vec<_> = app
            .keep
            .iter()
            .map(|r| (r.reference.as_str(), r.reason.as_str()))
            .collect();
        assert_eq!(
            keep,
            [
                ("r/app:sha-5", "newest"),
                ("r/app:sha-4", "newest"),
                ("r/app:rollback", "protected"),
                ("r/app:sha-0", "in_use")
            ]
        );
        let remove: Vec<_> = app.remove.iter().map(|r| r.reference.as_str()).collect();
        assert_eq!(remove, ["r/app:sha-3", "r/app:sha-2", "r/app:sha-1"]);
        // a2 keeps its rollback tag, so only a3, a1 and the dangling image free space.
        assert_eq!(p.reclaim, 300);
        assert_eq!(p.remove_count, 4);
        assert!(p
            .repos
            .iter()
            .find(|r| r.repo == "other")
            .unwrap()
            .remove
            .is_empty());
    }
}
