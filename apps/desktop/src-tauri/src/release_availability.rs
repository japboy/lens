//! Public release discovery. The transport is isolated from the presentation state so a
//! signed updater can replace it without changing the About window or tray projection.

use std::{
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const LATEST_RELEASE: &str = "https://api.github.com/repos/japboy/lens/releases/latest";
const RELEASE_TAG_BASE: &str = "https://github.com/japboy/lens/releases/tag/";
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const ORDINARY_FAILURE_COOLDOWN: Duration = Duration::from_secs(15);
const RATE_LIMIT_FALLBACK: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    Idle,
    Checking,
    Current,
    Available,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ReleaseAvailabilitySnapshot {
    pub(crate) revision: u64,
    pub(crate) stage: Stage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) release_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retry_after_epoch_ms: Option<u64>,
}

impl Default for ReleaseAvailabilitySnapshot {
    fn default() -> Self {
        Self {
            revision: 0,
            stage: Stage::Idle,
            version: None,
            release_url: None,
            retry_after_epoch_ms: None,
        }
    }
}

pub(crate) struct ReleaseAvailability {
    snapshot: Mutex<ReleaseAvailabilitySnapshot>,
    client: reqwest::Client,
}

impl Default for ReleaseAvailability {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(ReleaseAvailabilitySnapshot::default()),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
}

fn canonical_tag(tag: &str) -> Result<Version, String> {
    let raw = tag
        .strip_prefix('v')
        .ok_or_else(|| "Release tag must start with v".to_string())?;
    let version = Version::parse(raw).map_err(|error| error.to_string())?;
    if version.to_string() != raw || !version.pre.is_empty() || !version.build.is_empty() {
        return Err("Release tag must be a canonical stable vX.Y.Z version".into());
    }
    Ok(version)
}

fn classify_release(
    current: &Version,
    latest: LatestRelease,
) -> Result<ReleaseAvailabilitySnapshot, String> {
    let release = canonical_tag(&latest.tag_name)?;
    if release <= *current {
        return Ok(ReleaseAvailabilitySnapshot {
            stage: Stage::Current,
            ..Default::default()
        });
    }
    Ok(ReleaseAvailabilitySnapshot {
        stage: Stage::Available,
        version: Some(release.to_string()),
        release_url: Some(format!("{RELEASE_TAG_BASE}{}", latest.tag_name)),
        ..Default::default()
    })
}

struct CheckFailure {
    message: String,
    retry_after_epoch_ms: Option<u64>,
}

impl CheckFailure {
    fn ordinary(message: String) -> Self {
        Self {
            message,
            retry_after_epoch_ms: None,
        }
    }
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn header_u64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.parse().ok()
}

fn rate_limit_deadline(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    now_ms: u64,
) -> Option<u64> {
    if status != reqwest::StatusCode::FORBIDDEN && status != reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return None;
    }
    // GitHub documents Retry-After in seconds and x-ratelimit-reset in UTC epoch seconds.
    // When both are supplied, wait for both constraints; a one-second margin avoids a
    // request reaching GitHub just before the reset boundary.
    let retry_after = header_u64(headers, "retry-after")
        .map(|seconds| now_ms.saturating_add(seconds.saturating_mul(1000)));
    let reset = (header_u64(headers, "x-ratelimit-remaining") == Some(0))
        .then(|| header_u64(headers, "x-ratelimit-reset"))
        .flatten()
        .map(|seconds| seconds.saturating_mul(1000).saturating_add(1000));
    Some(
        retry_after
            .into_iter()
            .chain(reset)
            .max()
            .unwrap_or_else(|| now_ms.saturating_add(RATE_LIMIT_FALLBACK.as_millis() as u64)),
    )
}

async fn fetch_release(client: &reqwest::Client) -> Result<LatestRelease, CheckFailure> {
    let response = client
        .get(LATEST_RELEASE)
        .header(reqwest::header::USER_AGENT, "Lens desktop")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|error| CheckFailure::ordinary(error.to_string()))?;
    if !response.status().is_success() {
        return Err(CheckFailure {
            message: format!("GitHub latest release returned HTTP {}", response.status()),
            retry_after_epoch_ms: rate_limit_deadline(
                response.status(),
                response.headers(),
                now_epoch_ms(),
            ),
        });
    }
    response
        .json::<LatestRelease>()
        .await
        .map_err(|error| CheckFailure::ordinary(error.to_string()))
}

impl ReleaseAvailability {
    fn snapshot(&self) -> Result<ReleaseAvailabilitySnapshot, String> {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| "Release availability lock is poisoned".into())
    }

    fn begin(&self, now_ms: u64) -> Result<Option<ReleaseAvailabilitySnapshot>, String> {
        let mut snapshot = self
            .snapshot
            .lock()
            .map_err(|_| "Release availability lock is poisoned".to_string())?;
        if snapshot.stage == Stage::Checking
            || snapshot
                .retry_after_epoch_ms
                .is_some_and(|deadline| now_ms < deadline)
        {
            return Ok(None);
        }
        snapshot.revision += 1;
        snapshot.stage = Stage::Checking;
        snapshot.version = None;
        snapshot.release_url = None;
        snapshot.retry_after_epoch_ms = None;
        Ok(Some(snapshot.clone()))
    }

    fn finish(
        &self,
        result: Result<ReleaseAvailabilitySnapshot, CheckFailure>,
        now_ms: u64,
    ) -> Result<(), String> {
        let mut snapshot = self
            .snapshot
            .lock()
            .map_err(|_| "Release availability lock is poisoned".to_string())?;
        snapshot.revision += 1;
        let mut next = result.unwrap_or_else(|error| {
            eprintln!("Unable to check Lens release: {}", error.message);
            ReleaseAvailabilitySnapshot {
                stage: Stage::Failed,
                retry_after_epoch_ms: Some(error.retry_after_epoch_ms.unwrap_or_else(|| {
                    now_ms.saturating_add(ORDINARY_FAILURE_COOLDOWN.as_millis() as u64)
                })),
                ..Default::default()
            }
        });
        next.revision = snapshot.revision;
        *snapshot = next;
        Ok(())
    }
}

fn publish<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Err(error) = crate::ui::sync_about_menu(app) {
        eprintln!("Unable to update About menu: {error}");
    }
}

fn request_check<R: tauri::Runtime>(
    app: AppHandle<R>,
) -> Result<ReleaseAvailabilitySnapshot, String> {
    let availability = app.state::<ReleaseAvailability>();
    let Some(snapshot) = availability.begin(now_epoch_ms())? else {
        return availability.snapshot();
    };
    publish(&app);
    tauri::async_runtime::spawn(async move {
        let current = app.package_info().version.clone();
        let availability = app.state::<ReleaseAvailability>();
        let result = fetch_release(&availability.client)
            .await
            .and_then(|release| {
                classify_release(&current, release).map_err(CheckFailure::ordinary)
            });
        if let Err(error) = availability.finish(result, now_epoch_ms()) {
            eprintln!("Unable to publish release availability: {error}");
            return;
        }
        publish(&app);
    });
    Ok(snapshot)
}

pub(crate) fn start<R: tauri::Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = request_check(app.clone()) {
            eprintln!("Unable to begin release check: {error}");
        }
        loop {
            tokio::time::sleep(CHECK_INTERVAL).await;
            if let Err(error) = request_check(app.clone()) {
                eprintln!("Unable to begin release check: {error}");
            }
        }
    });
}

#[tauri::command]
pub(crate) fn get_release_availability<R: tauri::Runtime>(
    app: AppHandle<R>,
) -> Result<ReleaseAvailabilitySnapshot, String> {
    app.state::<ReleaseAvailability>().snapshot()
}

#[tauri::command]
pub(crate) fn retry_release_availability_check<R: tauri::Runtime>(
    app: AppHandle<R>,
) -> Result<ReleaseAvailabilitySnapshot, String> {
    request_check(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_canonical_stable_tags_are_accepted() {
        for tag in ["v1.2.3", "v0.0.0", "v12.345.678"] {
            assert!(canonical_tag(tag).is_ok(), "{tag}");
        }
        for tag in [
            "1.2.3",
            "v1.2",
            "v01.2.3",
            "v1.2.3-rc.1",
            "v1.2.3+build",
            "v1.2.3/evil",
        ] {
            assert!(canonical_tag(tag).is_err(), "{tag}");
        }
    }

    #[test]
    fn compares_versions_numerically_and_keeps_tag_url_canonical() {
        let current = Version::parse("1.9.0").unwrap();
        let available = classify_release(
            &current,
            LatestRelease {
                tag_name: "v1.10.0".into(),
            },
        )
        .unwrap();
        assert_eq!(available.stage, Stage::Available);
        assert_eq!(available.version.as_deref(), Some("1.10.0"));
        assert_eq!(
            available.release_url.as_deref(),
            Some("https://github.com/japboy/lens/releases/tag/v1.10.0")
        );
        let same = classify_release(
            &current,
            LatestRelease {
                tag_name: "v1.9.0".into(),
            },
        )
        .unwrap();
        assert_eq!(same.stage, Stage::Current);
        let older = classify_release(
            &current,
            LatestRelease {
                tag_name: "v1.8.9".into(),
            },
        )
        .unwrap();
        assert_eq!(older.stage, Stage::Current);
    }

    #[test]
    fn concurrent_check_requests_do_not_create_another_flight() {
        let state = ReleaseAvailability::default();
        assert_eq!(state.begin(1_000).unwrap().unwrap().revision, 1);
        assert!(state.begin(1_001).unwrap().is_none());
        state
            .finish(Err(CheckFailure::ordinary("offline".into())), 2_000)
            .unwrap();
        let failed = state.snapshot().unwrap();
        assert_eq!(failed.stage, Stage::Failed);
        assert_eq!(failed.retry_after_epoch_ms, Some(17_000));
        assert!(state.begin(16_999).unwrap().is_none());
        assert_eq!(state.begin(17_000).unwrap().unwrap().revision, 3);
    }

    #[test]
    fn rate_limit_headers_set_the_next_admissible_request_time() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "120".parse().unwrap());
        assert_eq!(
            rate_limit_deadline(reqwest::StatusCode::TOO_MANY_REQUESTS, &headers, 1_000),
            Some(121_000)
        );
        headers.insert("x-ratelimit-remaining", "0".parse().unwrap());
        headers.insert("x-ratelimit-reset", "200".parse().unwrap());
        assert_eq!(
            rate_limit_deadline(reqwest::StatusCode::FORBIDDEN, &headers, 1_000),
            Some(201_000)
        );
        headers.remove("retry-after");
        assert_eq!(
            rate_limit_deadline(reqwest::StatusCode::FORBIDDEN, &headers, 1_000),
            Some(201_000)
        );
        headers.clear();
        assert_eq!(
            rate_limit_deadline(reqwest::StatusCode::TOO_MANY_REQUESTS, &headers, 1_000),
            Some(61_000)
        );
        assert_eq!(
            rate_limit_deadline(reqwest::StatusCode::NOT_FOUND, &headers, 1_000),
            None
        );
    }

    #[test]
    fn rate_limit_cooldown_blocks_both_periodic_and_manual_admission() {
        let state = ReleaseAvailability::default();
        assert!(state.begin(1_000).unwrap().is_some());
        state
            .finish(
                Err(CheckFailure {
                    message: "rate limited".into(),
                    retry_after_epoch_ms: Some(100_000),
                }),
                2_000,
            )
            .unwrap();
        assert_eq!(
            state.snapshot().unwrap().retry_after_epoch_ms,
            Some(100_000)
        );
        assert!(state.begin(99_999).unwrap().is_none());
        assert!(state.begin(100_000).unwrap().is_some());
    }
}
