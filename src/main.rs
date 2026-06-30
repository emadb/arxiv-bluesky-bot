mod arxiv;
mod bluesky;
mod config;
mod format;

use anyhow::{anyhow, Result};
use chrono::Utc;
use chrono_tz::Tz;
use std::time::Duration;

use arxiv::{current_window_index, fetch_papers, filter_papers, select_window, FilterOptions};
use bluesky::BlueskyClient;
use config::Config;
use format::compose_post;

/// Today's date as YYYY-MM-DD in the given timezone.
fn today_in_timezone(time_zone: &str) -> Result<String> {
    let tz: Tz = time_zone
        .parse()
        .map_err(|_| anyhow!("invalid timezone: {time_zone}"))?;
    Ok(Utc::now().with_timezone(&tz).format("%Y-%m-%d").to_string())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::from_env()?;

    // Default target = today in arXiv's timezone. Because the feed refreshes at
    // 00:00 US/Eastern and we run a few hours later, "today" equals the date
    // stamped on the batch that just appeared. On weekends/holidays the feed is
    // empty (or stale-dated), so nothing matches and we post nothing — which is
    // exactly why no database is needed to avoid duplicates.
    let target_date = match &cfg.target_date {
        Some(d) => d.clone(),
        None => today_in_timezone(&cfg.time_zone)?,
    };

    println!(
        "[arxiv→bsky] categories={} date={} types=[{}] dryRun={}",
        cfg.categories,
        target_date,
        cfg.announce_types.join(","),
        cfg.dry_run
    );

    let all = fetch_papers(&cfg.categories, &cfg.time_zone).await?;
    println!("[arxiv→bsky] feed returned {} item(s)", all.len());

    let matched = filter_papers(
        &all,
        &FilterOptions {
            announce_types: cfg.announce_types.clone(),
            target_date: target_date.clone(),
        },
    );
    println!(
        "[arxiv→bsky] {} match date+type after de-dup",
        matched.len()
    );

    // Spread one day's batch across the day: each scheduled run posts only the
    // slice assigned to the current time-window. A paper maps to exactly one
    // window, so across all runs each is posted exactly once — no state needed.
    let tz: Tz = cfg
        .time_zone
        .parse()
        .map_err(|_| anyhow!("invalid timezone: {}", cfg.time_zone))?;
    let window_index = current_window_index(Utc::now(), &tz, cfg.post_windows);
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
        return Ok(());
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

    let client = if cfg.dry_run {
        None
    } else {
        Some(BlueskyClient::login(&cfg.service, &cfg.handle, &cfg.app_password).await?)
    };

    let mut posted = 0u32;
    let mut failed = 0u32;
    for paper in to_post {
        let composed = compose_post(paper);

        let Some(client) = &client else {
            println!(
                "\n--- DRY RUN [{}] ({}) ---\n{}\n[card → {}]",
                paper.id, paper.announce_type, composed.text, composed.embed.uri
            );
            continue;
        };

        match client.post(&composed).await {
            Ok(uri) => {
                posted += 1;
                println!("[arxiv→bsky] posted {} → {}", paper.id, uri);
            }
            Err(err) => {
                failed += 1;
                eprintln!("[arxiv→bsky] FAILED {}: {:#}", paper.id, err);
            }
        }
        tokio::time::sleep(Duration::from_millis(cfg.post_delay_ms)).await;
    }

    println!(
        "[arxiv→bsky] done. posted={} failed={} considered={}",
        posted,
        failed,
        to_post.len()
    );

    // Surface partial failures to the GitHub Actions run without aborting posts.
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
