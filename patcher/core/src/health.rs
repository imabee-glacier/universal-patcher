//! Service health for the website: probes the public origin the way users reach it and records
//! a timestamped result. Release metadata is never taken as evidence that the service is up.
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

/// Metadata expiring sooner than this is reported as a disruption, matching the expiry alert.
pub const EXPIRY_MARGIN_HOURS: i64 = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
}

/// `operational` when every check passes, `unavailable` when the front itself does not answer,
/// otherwise `degraded`.
pub fn status(checks: &[Check]) -> &'static str {
    if checks.iter().all(|c| c.ok) {
        "operational"
    } else if checks.iter().any(|c| c.name == "front" && !c.ok) {
        "unavailable"
    } else {
        "degraded"
    }
}

async fn get(client: &reqwest::Client, url: String) -> Option<Vec<u8>> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.bytes().await.ok().map(|b| b.to_vec())
}

fn fresh(timestamp: &[u8], now: jiff::Timestamp) -> bool {
    let margin = jiff::SignedDuration::from_hours(EXPIRY_MARGIN_HOURS);
    crate::strict_json::parse(timestamp)
        .ok()
        .and_then(|v: Value| v.pointer("/signed/expires")?.as_str()?.parse().ok())
        .is_some_and(|expires: jiff::Timestamp| expires > now + margin)
}

/// Probes `origin` and returns the checks in a fixed order.
pub async fn probe(origin: &str, now: jiff::Timestamp) -> Result<Vec<Check>> {
    crate::distribution::check_origin(origin)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let front = get(&client, format!("{origin}healthz")).await.is_some();
    let installers = get(&client, format!("{origin}install.sh")).await.is_some()
        && get(&client, format!("{origin}install.ps1")).await.is_some();
    let latest = get(&client, format!("{origin}app/bootstrap/latest"))
        .await
        .and_then(|b| String::from_utf8(b).ok())
        .is_some_and(|v| crate::model::parse_version(v.trim()).is_ok());
    let app_metadata = get(&client, format!("{origin}app/metadata/timestamp.json"))
        .await
        .is_some_and(|b| fresh(&b, now));
    Ok(vec![
        Check {
            name: "front",
            ok: front,
        },
        Check {
            name: "installers",
            ok: installers,
        },
        Check {
            name: "patcher-release",
            ok: latest,
        },
        Check {
            name: "patcher-metadata",
            ok: app_metadata,
        },
    ])
}

/// Probes `origin` and writes the website's `health.json` to `output`.
pub async fn write(origin: &str, output: &Path) -> Result<&'static str> {
    let now = jiff::Timestamp::now();
    let checks = probe(origin, now).await?;
    let status = status(&checks);
    let report = json!({
        "schema": 1,
        "status": status,
        "checked_at": now.to_string(),
        "checks": checks.iter().map(|c| json!({ "name": c.name, "ok": c.ok })).collect::<Vec<_>>(),
    });
    crate::storage::atomic_write_public(output, &serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("Writing {}", output.display()))?;
    Ok(status)
}
