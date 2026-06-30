//! Configuration is read entirely from environment variables so the same code
//! runs locally (via a `.env` you source yourself) and in GitHub Actions (via
//! repo Secrets/Variables). Nothing is persisted between runs.

use anyhow::{anyhow, Result};
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    /// Bluesky handle of the bot account, e.g. "arxiv-cs.bsky.social".
    pub handle: String,
    /// Bluesky APP PASSWORD (never the account's real password).
    pub app_password: String,
    /// PDS host. Almost always https://bsky.social.
    pub service: String,
    /// arXiv feed path, e.g. "cs.AI" or "cs.AI+cs.LG" (max 2000 results).
    pub categories: String,
    /// Which announce types to post. Default: only brand-new papers.
    pub announce_types: Vec<String>,
    /// Force a specific announcement date (YYYY-MM-DD). None = "today" in tz.
    pub target_date: Option<String>,
    /// Safety cap on how many posts a single run may emit.
    pub max_posts: usize,
    /// Delay between posts, to stay friendly with rate limits.
    pub post_delay_ms: u64,
    /// If true, log what would be posted but don't touch Bluesky.
    pub dry_run: bool,
    /// Timezone arXiv stamps its announcements in.
    pub time_zone: String,
    /// Number of time-windows to spread one day's batch across. 1 = post the
    /// whole batch in a single run (legacy behavior). >1 = each run posts only
    /// the slice of the batch assigned to the current window.
    pub post_windows: u32,
}

fn required(name: &str) -> Result<String> {
    match env::var(name) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => Err(anyhow!("Missing required environment variable: {name}")),
    }
}

/// Optional string: trimmed value, or `None` when unset/blank.
fn opt(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn bool_env(name: &str, fallback: bool) -> bool {
    match opt(name) {
        Some(v) => matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        None => fallback,
    }
}

/// Parse an integer env var, falling back on a missing/non-numeric value
/// (mirrors the original `Number.isFinite` guard).
fn int_env<T: std::str::FromStr>(name: &str, fallback: T) -> T {
    match opt(name) {
        Some(v) => v.parse::<T>().unwrap_or(fallback),
        None => fallback,
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let announce_types = opt("ARXIV_ANNOUNCE_TYPES")
            .unwrap_or_else(|| "new".to_string())
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Config {
            handle: required("BSKY_HANDLE")?,
            app_password: required("BSKY_APP_PASSWORD")?,
            service: opt("BSKY_SERVICE").unwrap_or_else(|| "https://bsky.social".to_string()),
            categories: opt("ARXIV_CATEGORIES").unwrap_or_else(|| "cs.AI".to_string()),
            announce_types,
            target_date: opt("TARGET_DATE"),
            max_posts: int_env("MAX_POSTS", 50),
            post_delay_ms: int_env("POST_DELAY_MS", 1500),
            dry_run: bool_env("DRY_RUN", false),
            time_zone: opt("ARXIV_TIMEZONE").unwrap_or_else(|| "America/New_York".to_string()),
            post_windows: int_env("POST_WINDOWS", 6),
        })
    }
}
