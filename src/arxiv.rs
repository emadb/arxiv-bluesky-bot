//! Fetch and parse the arXiv RSS feed, then filter and window a day's batch.
//!
//! The network boundary (`fetch_feed_xml`) is split from the pure parse
//! (`parse_feed`) so all behavior below the fetch is testable offline.

use anyhow::{Context, Result};
use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Tz;
use rss::Channel;
use std::collections::HashSet;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArxivPaper {
    /// Canonical id without version suffix, e.g. "2506.01250".
    pub id: String,
    pub title: String,
    pub authors: String,
    pub abstract_text: String,
    /// Abstract landing page, e.g. https://arxiv.org/abs/2506.01250
    pub link: String,
    /// "new" | "cross" | "replace" | "replace-cross" | ...
    pub announce_type: String,
    /// Announcement date as YYYY-MM-DD in the arXiv timezone.
    pub announce_date: String,
    pub categories: Vec<String>,
}

/// Collapse all runs of whitespace to single spaces and trim (mirrors the
/// `replace(/\s+/g, " ").trim()` used throughout the original).
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Strip a trailing `vN` version suffix (`v\d+$`, case-insensitive).
fn strip_version(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i < bytes.len() && i > 0 && (bytes[i - 1] == b'v' || bytes[i - 1] == b'V') {
        return s[..i - 1].to_string();
    }
    s.to_string()
}

/// "oai:arXiv.org:2506.01250v1" -> "2506.01250" (also handles old-style ids).
fn parse_id(guid: Option<&str>, link: Option<&str>) -> String {
    if let Some(g) = guid {
        let last = g.rsplit(':').next().unwrap_or(g);
        return strip_version(last);
    }
    if let Some(l) = link {
        if let Some(idx) = l.find("abs/") {
            return strip_version(&l[idx + 4..]);
        }
        return l.to_string();
    }
    "unknown".to_string()
}

/// Pull the abstract out of the description blob, dropping the
/// "arXiv:.. Announce Type:" header.
fn parse_abstract(description: Option<&str>) -> String {
    let Some(desc) = description else {
        return String::new();
    };
    let marker = "Abstract:";
    let raw = match desc.find(marker) {
        Some(idx) => &desc[idx + marker.len()..],
        None => desc,
    };
    collapse_whitespace(raw)
}

/// Format a `pubDate` (RFC 822/2822, with an RFC 3339 fallback) as YYYY-MM-DD
/// in the given timezone — the announcement *date*.
fn date_in_timezone(raw: Option<&str>, tz: &Tz) -> String {
    let Some(s) = raw else {
        return String::new();
    };
    let parsed = DateTime::parse_from_rfc2822(s).or_else(|_| DateTime::parse_from_rfc3339(s));
    match parsed {
        Ok(dt) => dt.with_timezone(tz).format("%Y-%m-%d").to_string(),
        Err(_) => String::new(),
    }
}

/// Read the custom `<arxiv:announce_type>` element from the item's namespaced
/// extension map (parsed from the dedicated element, not the description blob).
fn announce_type(item: &rss::Item) -> String {
    item.extensions()
        .get("arxiv")
        .and_then(|m| m.get("announce_type"))
        .and_then(|v| v.first())
        .and_then(|e| e.value())
        .unwrap_or("")
        .trim()
        .to_lowercase()
}

/// `dc:creator` — a single comma-separated author string. Read from the Dublin
/// Core extension, falling back to the raw namespaced extension map.
fn creators(item: &rss::Item) -> String {
    if let Some(dc) = item.dublin_core_ext() {
        if !dc.creators().is_empty() {
            return collapse_whitespace(&dc.creators().join(", "));
        }
    }
    let joined = item
        .extensions()
        .get("dc")
        .and_then(|m| m.get("creator"))
        .map(|v| {
            v.iter()
                .filter_map(|e| e.value())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    collapse_whitespace(&joined)
}

/// Parse a feed XML document into papers. Pure: no network, no clock.
pub fn parse_feed(xml: &str, tz: &Tz) -> Result<Vec<ArxivPaper>> {
    let channel = Channel::read_from(xml.as_bytes()).context("failed to parse RSS feed")?;

    Ok(channel
        .items()
        .iter()
        .map(|item| {
            let id = parse_id(item.guid().map(|g| g.value()), item.link());
            let link = item
                .link()
                .map(|l| l.to_string())
                .unwrap_or_else(|| format!("https://arxiv.org/abs/{id}"));
            ArxivPaper {
                title: collapse_whitespace(item.title().unwrap_or("")),
                authors: creators(item),
                abstract_text: parse_abstract(item.description()),
                link,
                announce_type: announce_type(item),
                announce_date: date_in_timezone(item.pub_date(), tz),
                categories: item
                    .categories()
                    .iter()
                    .map(|c| c.name().to_string())
                    .collect(),
                id,
            }
        })
        .collect())
}

/// Fetch the live feed (network boundary) and parse it.
pub async fn fetch_papers(categories: &str, tz: &Tz) -> Result<Vec<ArxivPaper>> {
    let xml = fetch_feed_xml(categories).await?;
    parse_feed(&xml, tz)
}

async fn fetch_feed_xml(categories: &str) -> Result<String> {
    let url = format!("https://rss.arxiv.org/rss/{categories}");
    let client = reqwest::Client::builder()
        .user_agent("arxiv-bluesky-bot (+https://github.com/)")
        .timeout(Duration::from_secs(30))
        .build()?;
    let body = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(body)
}

/// First posting window starts at `WINDOW_START_HOUR` and each subsequent
/// window opens `WINDOW_SPACING_HOURS` later, both in the arXiv timezone. With 6
/// windows this yields slots at 06, 09, 12, 15, 18, 21 US/Eastern.
///
/// INVARIANT: these MUST stay in sync with the cron schedule in
/// `.github/workflows/post.yml` — one scheduled run per slot.
const WINDOW_START_HOUR: i64 = 6;
const WINDOW_SPACING_HOURS: i64 = 3;

/// Which posting window `now` falls in, as an index in `[0, window_count-1]`.
/// Derived from the *actual* wall-clock hour in `tz` (not from which cron
/// fired), so it tolerates cron jitter and DST shifts. Returns 0 when
/// `window_count <= 1` (single-batch mode).
pub fn current_window_index(now: DateTime<Utc>, tz: &Tz, window_count: u32) -> usize {
    if window_count <= 1 {
        return 0;
    }
    let hour = now.with_timezone(tz).hour() as i64;
    let raw = (hour - WINDOW_START_HOUR).div_euclid(WINDOW_SPACING_HOURS);
    raw.clamp(0, window_count as i64 - 1) as usize
}

/// Tiny deterministic string hash (FNV-1a, 32-bit). Iterates UTF-16 code units
/// to match the original `charCodeAt`/`Math.imul` semantics exactly.
fn hash_id(id: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for cu in id.encode_utf16() {
        h ^= cu as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Partition a day's filtered papers into `window_count` disjoint slices and
/// return the slice for `index`. Each paper maps to exactly one window via a
/// stable hash of its id, so across all indices every paper is posted exactly
/// once. Returns the input unchanged when `window_count <= 1`.
pub fn select_window(papers: &[ArxivPaper], index: usize, window_count: u32) -> Vec<ArxivPaper> {
    if window_count <= 1 {
        return papers.to_vec();
    }
    papers
        .iter()
        .filter(|p| (hash_id(&p.id) % window_count) as usize == index)
        .cloned()
        .collect()
}

pub struct FilterOptions {
    /// Allowed announce types, already lowercased/canonical. Empty = any type.
    pub announce_types: HashSet<String>,
    pub target_date: String,
}

/// Keep only papers (a) announced on the target date and (b) of a wanted
/// announce type, de-duplicated by id (a paper can appear twice when several
/// categories are combined with "+").
pub fn filter_papers(papers: &[ArxivPaper], opts: &FilterOptions) -> Vec<ArxivPaper> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();

    for p in papers {
        if p.announce_date != opts.target_date {
            continue;
        }
        if !opts.announce_types.is_empty() && !opts.announce_types.contains(&p.announce_type) {
            continue;
        }
        // `insert` returns false when the id was already present.
        if !seen.insert(&p.id) {
            continue;
        }
        out.push(p.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paper(id: &str, date: &str, ty: &str) -> ArxivPaper {
        ArxivPaper {
            id: id.to_string(),
            title: format!("Title {id}"),
            authors: "A, B".to_string(),
            abstract_text: "abs".to_string(),
            link: format!("https://arxiv.org/abs/{id}"),
            announce_type: ty.to_string(),
            announce_date: date.to_string(),
            categories: vec!["cs.AI".to_string()],
        }
    }

    #[test]
    fn strip_version_cases() {
        assert_eq!(strip_version("2506.01250v1"), "2506.01250");
        assert_eq!(strip_version("2506.01250V12"), "2506.01250");
        assert_eq!(strip_version("2506.01250"), "2506.01250");
        assert_eq!(strip_version("cs/0501001v3"), "cs/0501001");
        // "v" with no digits, or digits with no "v": unchanged.
        assert_eq!(strip_version("abcv"), "abcv");
        assert_eq!(strip_version("abc12"), "abc12");
    }

    #[test]
    fn parse_id_from_guid_and_link() {
        assert_eq!(
            parse_id(Some("oai:arXiv.org:2506.01250v1"), None),
            "2506.01250"
        );
        assert_eq!(
            parse_id(None, Some("https://arxiv.org/abs/2506.01250v2")),
            "2506.01250"
        );
        assert_eq!(parse_id(None, None), "unknown");
    }

    #[test]
    fn parse_abstract_drops_header() {
        let blob = "arXiv:2506.01250v1 Announce Type: new\nAbstract:  hello   world ";
        assert_eq!(parse_abstract(Some(blob)), "hello world");
        assert_eq!(parse_abstract(Some("no marker here")), "no marker here");
        assert_eq!(parse_abstract(None), "");
    }

    #[test]
    fn date_in_timezone_uses_eastern() {
        let tz: Tz = "America/New_York".parse().unwrap();
        // 03:30 UTC is still the previous calendar day in US/Eastern.
        assert_eq!(
            date_in_timezone(Some("Mon, 30 Jun 2025 03:30:00 GMT"), &tz),
            "2025-06-29"
        );
        assert_eq!(
            date_in_timezone(Some("Mon, 30 Jun 2025 12:00:00 GMT"), &tz),
            "2025-06-30"
        );
    }

    const SAMPLE_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:arxiv="http://arxiv.org/schemas/atom" version="2.0">
  <channel>
    <title>cs.AI updates on arXiv.org</title>
    <link>https://arxiv.org/</link>
    <description>cs.AI updates</description>
    <item>
      <title>A New Result
        on Things</title>
      <link>https://arxiv.org/abs/2506.01250</link>
      <description>arXiv:2506.01250v1 Announce Type: new
Abstract: We show   that things  are   true.</description>
      <guid isPermaLink="false">oai:arXiv.org:2506.01250v1</guid>
      <category>cs.AI</category>
      <category>cs.LG</category>
      <pubDate>Mon, 30 Jun 2025 04:00:00 +0000</pubDate>
      <arxiv:announce_type>new</arxiv:announce_type>
      <dc:creator>Ada Lovelace, Alan Turing</dc:creator>
    </item>
    <item>
      <title>A Cross Listing</title>
      <link>https://arxiv.org/abs/2506.09999</link>
      <description>arXiv:2506.09999v2 Announce Type: cross
Abstract: Cross stuff.</description>
      <guid isPermaLink="false">oai:arXiv.org:2506.09999v2</guid>
      <category>cs.AI</category>
      <pubDate>Mon, 30 Jun 2025 04:00:00 +0000</pubDate>
      <arxiv:announce_type>cross</arxiv:announce_type>
      <dc:creator>Grace Hopper</dc:creator>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn parse_feed_extracts_all_fields() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let papers = parse_feed(SAMPLE_FEED, &tz).unwrap();
        assert_eq!(papers.len(), 2);

        let p = &papers[0];
        assert_eq!(p.id, "2506.01250"); // guid segment, vN stripped
        assert_eq!(p.title, "A New Result on Things"); // whitespace collapsed
        assert_eq!(p.authors, "Ada Lovelace, Alan Turing"); // dc:creator
        assert_eq!(p.abstract_text, "We show that things are true."); // after Abstract:
        assert_eq!(p.link, "https://arxiv.org/abs/2506.01250");
        assert_eq!(p.announce_type, "new"); // arxiv:announce_type extension
                                            // 04:00 UTC = 00:00 US/Eastern (EDT) -> still 2025-06-30.
        assert_eq!(p.announce_date, "2025-06-30");
        assert_eq!(p.categories, vec!["cs.AI".to_string(), "cs.LG".to_string()]);

        assert_eq!(papers[1].id, "2506.09999");
        assert_eq!(papers[1].announce_type, "cross");
        assert_eq!(papers[1].authors, "Grace Hopper");
    }

    #[test]
    fn hash_id_matches_reference_implementation() {
        // Reference FNV-1a (UTF-16) values, mirroring the TypeScript hashId.
        let cases: &[(&str, u32)] = &[
            ("2506.01250", 3_145_968_922),
            ("2506.01251", 3_162_746_541),
            ("hello", 1_335_831_723),
            ("2401.99999", 3_040_828_277),
            ("2312.00001v2", 2_173_715_212),
            ("abc", 440_920_331),
        ];
        for (id, expected) in cases {
            assert_eq!(hash_id(id), *expected, "hash mismatch for {id}");
        }
    }

    #[test]
    fn filter_by_date_type_and_dedup() {
        let papers = vec![
            paper("1", "2025-06-30", "new"),
            paper("2", "2025-06-29", "new"),   // wrong date
            paper("3", "2025-06-30", "cross"), // wrong type
            paper("1", "2025-06-30", "new"),   // duplicate id
            paper("4", "2025-06-30", "new"),
        ];
        let out = filter_papers(
            &papers,
            &FilterOptions {
                announce_types: HashSet::from(["new".to_string()]),
                target_date: "2025-06-30".to_string(),
            },
        );
        let ids: Vec<&str> = out.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["1", "4"]);
    }

    #[test]
    fn empty_announce_types_keeps_all_types() {
        let papers = vec![paper("1", "2025-06-30", "cross")];
        let out = filter_papers(
            &papers,
            &FilterOptions {
                announce_types: HashSet::new(),
                target_date: "2025-06-30".to_string(),
            },
        );
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn windows_form_a_disjoint_partition() {
        let papers: Vec<ArxivPaper> = (0..500)
            .map(|i| paper(&format!("2506.{i:05}"), "2025-06-30", "new"))
            .collect();
        let window_count = 6;

        let mut union: Vec<String> = Vec::new();
        for index in 0..window_count as usize {
            let slice = select_window(&papers, index, window_count);
            for p in &slice {
                union.push(p.id.clone());
            }
        }
        // Every paper appears exactly once across all windows.
        union.sort();
        let mut expected: Vec<String> = papers.iter().map(|p| p.id.clone()).collect();
        expected.sort();
        assert_eq!(union, expected);
    }

    #[test]
    fn single_window_returns_everything() {
        let papers = vec![
            paper("1", "2025-06-30", "new"),
            paper("2", "2025-06-30", "new"),
        ];
        assert_eq!(select_window(&papers, 0, 1).len(), 2);
    }

    #[test]
    fn window_index_clamps_and_derives_from_hour() {
        let tz: Tz = "America/New_York".parse().unwrap();
        // 2025-06-30 is EDT (UTC-4). 10:00 UTC = 06:00 Eastern -> window 0.
        let at = |utc: &str| {
            DateTime::parse_from_rfc3339(utc)
                .unwrap()
                .with_timezone(&Utc)
        };
        assert_eq!(current_window_index(at("2025-06-30T10:00:00Z"), &tz, 6), 0); // 06 ET
        assert_eq!(current_window_index(at("2025-06-30T13:00:00Z"), &tz, 6), 1); // 09 ET
        assert_eq!(current_window_index(at("2025-06-30T08:00:00Z"), &tz, 6), 0); // 04 ET, clamps low
        assert_eq!(current_window_index(at("2025-07-01T03:00:00Z"), &tz, 6), 5); // 23 ET, clamps high
                                                                                 // single-batch mode always 0.
        assert_eq!(current_window_index(at("2025-06-30T13:00:00Z"), &tz, 1), 0);
    }
}
