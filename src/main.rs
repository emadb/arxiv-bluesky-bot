mod arxiv;
mod bluesky;
mod config;
mod format;

use anyhow::Result;
use chrono::Utc;
use chrono_tz::Tz;
use std::process::ExitCode;
use std::time::Duration;

use arxiv::{
    current_window_index, fetch_papers, filter_papers, select_window, ArxivPaper, FilterOptions,
};
use bluesky::BlueskyClient;
use config::Config;
use format::compose_post;

/// Today's date as YYYY-MM-DD in the given timezone.
fn today_in_timezone(tz: &Tz) -> String {
    Utc::now().with_timezone(tz).format("%Y-%m-%d").to_string()
}

#[derive(Default)]
struct PostStats {
    posted: u32,
    failed: u32,
}

/// Post every paper, attempting all of them even if some fail. Pacing between
/// posts keeps us friendly with rate limits.
async fn post_all(client: &BlueskyClient, papers: &[ArxivPaper], delay: Duration) -> PostStats {
    let mut stats = PostStats::default();
    for paper in papers {
        let composed = compose_post(paper);
        match client.post(&composed).await {
            Ok(uri) => {
                stats.posted += 1;
                println!("[arxiv→bsky] posted {} → {}", paper.id, uri);
            }
            Err(err) => {
                stats.failed += 1;
                eprintln!("[arxiv→bsky] FAILED {}: {:#}", paper.id, err);
            }
        }
        tokio::time::sleep(delay).await;
    }
    stats
}

/// Log what would be posted without touching Bluesky.
fn log_dry_run(papers: &[ArxivPaper]) {
    for paper in papers {
        let composed = compose_post(paper);
        println!(
            "\n--- DRY RUN [{}] ({}) ---\n{}\n[card → {}]",
            paper.id, paper.announce_type, composed.text, composed.embed.uri
        );
    }
}

#[tokio::main]
async fn main() -> Result<ExitCode> {
    let cfg = Config::from_env()?;

    // Default target = today in arXiv's timezone. Because the feed refreshes at
    // 00:00 US/Eastern and we run a few hours later, "today" equals the date
    // stamped on the batch that just appeared. On weekends/holidays the feed is
    // empty (or stale-dated), so nothing matches and we post nothing — which is
    // exactly why no database is needed to avoid duplicates.
    let target_date = cfg
        .target_date
        .clone()
        .unwrap_or_else(|| today_in_timezone(&cfg.time_zone));

    // Sorted so the log line is stable run-to-run (the set has no order).
    let mut types: Vec<&str> = cfg.announce_types.iter().map(String::as_str).collect();
    types.sort_unstable();

    println!(
        "[arxiv→bsky] categories={} date={} types=[{}] dryRun={}",
        cfg.categories,
        target_date,
        types.join(","),
        cfg.dry_run
    );

    let all = fetch_papers(&cfg.categories, &cfg.time_zone).await?;
    println!("[arxiv→bsky] feed returned {} item(s)", all.len());

    let matched = filter_papers(
        &all,
        &FilterOptions {
            announce_types: cfg.announce_types.clone(),
            target_date,
        },
    );
    println!(
        "[arxiv→bsky] {} match date+type after de-dup",
        matched.len()
    );

    // Spread one day's batch across the day: each scheduled run posts only the
    // slice assigned to the current time-window. A paper maps to exactly one
    // window, so across all runs each is posted exactly once — no state needed.
    let window_index = current_window_index(Utc::now(), &cfg.time_zone, cfg.post_windows);
    let papers = select_window(&matched, window_index, cfg.post_windows);
    if cfg.post_windows > 1 {
        println!(
            "[arxiv→bsky] window {}/{}: {} of {} paper(s) in this slice",
            window_index + 1,
            cfg.post_windows,
            papers.len(),
            matched.len()
        );
    }

    if papers.is_empty() {
        println!("[arxiv→bsky] nothing to post; exiting cleanly.");
        return Ok(ExitCode::SUCCESS);
    }

    let cap = cfg.max_posts.min(papers.len());
    if cap < papers.len() {
        eprintln!(
            "[arxiv→bsky] capping at MAX_POSTS={} ({} matched). Raise the cap or narrow categories.",
            cfg.max_posts,
            papers.len()
        );
    }
    let to_post = &papers[..cap];

    let stats = if cfg.dry_run {
        log_dry_run(to_post);
        PostStats::default()
    } else {
        let client = BlueskyClient::login(&cfg.service, &cfg.handle, &cfg.app_password).await?;
        post_all(&client, to_post, Duration::from_millis(cfg.post_delay_ms)).await
    };

    println!(
        "[arxiv→bsky] done. posted={} failed={} considered={}",
        stats.posted,
        stats.failed,
        to_post.len()
    );

    // Surface partial failures to the GitHub Actions run without aborting posts.
    Ok(if stats.failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}
