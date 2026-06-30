# CLAUDE.md

Context for AI coding agents working on this repository. Read this before
making changes. The companion `README.md` is for humans operating the bot;
this file is about *how the code works and why*, and what must not be broken.

## What this project is

A small Rust program that posts each day's **newly announced arXiv
papers** to a **Bluesky** account, run **once a day by GitHub Actions**. It
holds **no state** — no database, no committed cursor file, nothing. Avoiding
duplicate posts is achieved purely through arXiv feed semantics (see the
invariant below). Keep it that way unless explicitly asked to add persistence.

## The core invariant (do not break this)

Duplicate avoidance depends on a chain of facts about arXiv's RSS feeds:

1. Each feed is **rebuilt daily at 00:00 US/Eastern** and contains **only the
   latest announcement batch** — not a rolling history.
2. Every item carries an announcement date (`pubDate`) and an
   `arxiv:announce_type` (`new`, `cross`, `replace`, `replace-cross`).
3. The feed is **empty on Saturdays, Sundays, and arXiv holidays**.

The bot keeps items whose announcement date equals **today in US/Eastern** and
whose type is in the configured allow-list (default `new`), then posts them.

Why this is exactly-once without state:
- The GitHub job runs at **09:00 UTC**, a few hours after the 00:00 Eastern
  refresh, so `today` (US/Eastern, computed at run time) equals the date
  stamped on the fresh batch. One run per day ⇒ each batch posted once.
- Weekend/holiday runs match nothing (empty feed) ⇒ no-op, no error.
- A stale feed left over on a holiday is dated a *previous* day, so it won't
  match `today` ⇒ no re-post.

**Consequences for anyone editing this:**
- Do **not** change the cron times without re-checking the Eastern alignment.
  If a job runs *before* ~00:00 Eastern, `today` will be the *previous*
  batch's date and you'll either skip a day or (next day) re-post it.
- **Windowed posting:** the batch is dripped out over **6 daily runs** rather
  than one. Each run posts only the slice of the day's batch assigned to its
  time-window (`selectWindow` hashes each id to exactly one of N windows;
  `currentWindowIndex` picks the window from the live Eastern hour). Every paper
  still posts exactly once — slicing is a second filter *on top of* the
  `date == today` dedup, so the stateless invariant is unchanged. The six crons
  in `post.yml` **must stay in sync** with `WINDOW_START_HOUR` (6) and
  `WINDOW_SPACING_HOURS` (3) in `arxiv.rs` — one run per window. Set
  `POST_WINDOWS=1` to post a whole batch at once (the default for manual
  backfills).
- Do **not** loosen the date filter to "today OR yesterday" or "latest batch
  in feed" — both reintroduce duplicate risk on holidays without a database.
- The default target date is **today**, not yesterday, even though the README
  talks about "yesterday's papers." That's intentional: arXiv's batch
  announced on day *D* consists of papers *submitted* the day before, so
  "today's batch" already is "yesterday's submissions." `TARGET_DATE` overrides
  the default for manual backfills.

## arXiv feed facts (empirically verified — trust these over guesses)

Feed URL: `https://rss.arxiv.org/rss/<categories>` where `<categories>` is a
whole archive (`cs`), a subject class (`cs.AI`), or several joined with `+`
(`cs.AI+cs.LG`, max 2000 results). Spec:
<https://info.arxiv.org/help/rss_specifications.html>.

`rss` crate field mapping for arXiv items (checked against a real feed, not
assumed):

| Source XML | `rss` crate accessor | Notes |
|---|---|---|
| `<title>` | `item.title()` | |
| `<link>` | `item.link()` | abstract page, `https://arxiv.org/abs/<id>` |
| `<guid>` | `item.guid().value()` | `oai:arXiv.org:2506.01250v1` — id is the last `:` segment, strip trailing `vN` |
| `<pubDate>` | `item.pub_date()` | RFC 822/2822; parse with `chrono`, then format in `America/New_York` to get the announcement *date* |
| `<dc:creator>` | `item.dublin_core_ext().creators()` | comma-separated authors (Dublin Core extension; falls back to the raw `extensions()["dc"]["creator"]` map) |
| `<description>` | `item.description()` | blob: `arXiv:<id>vN Announce Type: <type>\nAbstract: <text>` |
| `<category>` (repeated) | `item.categories()` | slice of `Category` |
| `<arxiv:announce_type>` | `item.extensions()["arxiv"]["announce_type"]` | **custom namespace element**, read from the extension map |

`announce_type` is parsed from the dedicated element, **not** from the
description blob. The abstract is sliced out of `item.description()` after the
literal `Abstract:` marker.

## Architecture

```
src/
  config.rs    Config::from_env() → typed Config. The ONLY source of configuration.
  arxiv.rs     fetch_papers() = fetch_feed_xml() (network) + parse_feed() (pure) → Vec<ArxivPaper>; filter_papers() → date + type + dedup; current_window_index()/select_window() → drip one batch across N daily windows.
  format.rs    compose_post() → ComposedPost { text, embed: ExternalEmbed } within Bluesky's 300-grapheme limit.
  bluesky.rs   BlueskyClient::login() via bsky-sdk (atrium), returns a thin .post() wrapper.
  main.rs      #[tokio::main] async main(): load config → fetch → filter → window-slice → (dry-run log | login + post loop).
.github/workflows/post.yml   Daily cron + manual workflow_dispatch (target_date, dry_run).
```

Data flow is linear and side-effect-free until the post loop in `main.rs`. The
pure functions (`filter_papers`, `select_window`, `compose_post`, `parse_feed`
and parsing helpers) are the right place to unit-test; they need no network and
have `#[cfg(test)]` modules alongside them.

## Hard constraints baked into the code

- **Bluesky post text ≤ 300 graphemes.** Enforced in `format.rs` with
  `unicode-segmentation` (`graphemes(true)` — *not* `.len()` (bytes) or
  `.chars().count()` (code points)). The clickable link/title/abstract live in
  an `app.bsky.embed.external` card so the post text itself stays short.
- **App Password, never the real password.** `BSKY_APP_PASSWORD` is a Bluesky
  App Password. Never log it, never commit it, never hardcode it.
- **Volume.** A busy class (`cs.LG`, `cs.AI`) can be hundreds of papers per
  weekday. `MAX_POSTS` caps a run; `POST_DELAY_MS` paces the loop to stay
  within rate limits. Don't remove these guards.

## Conventions

- Rust 2021, **async on `tokio`** (`#[tokio::main]`). Single binary crate.
- **Crates:** `reqwest` (feed fetch), `rss` (parse), `chrono` + `chrono-tz`
  (IANA timezone dates), `unicode-segmentation` (graphemes), `bsky-sdk` /
  `atrium-api` (AT Protocol), `anyhow` (errors).
- **No committed build artifact.** Run with `cargo run` (or `--release`).
  `cargo build`/`cargo test`/`cargo clippy` must stay green.
- Logging is plain `println!`/`eprintln!` prefixed `[arxiv→bsky]`, written for
  the GitHub Actions run log. Keep partial-failure visibility: the process
  exits non-zero (`std::process::exit(1)`) if any post fails but still attempts
  every paper.

## Configuration surface (all via env)

Required: `BSKY_HANDLE`, `BSKY_APP_PASSWORD`.
Optional: `BSKY_SERVICE` (default `https://bsky.social`), `ARXIV_CATEGORIES`
(`cs.AI`), `ARXIV_ANNOUNCE_TYPES` (`new`), `MAX_POSTS` (50), `POST_DELAY_MS`
(1500), `ARXIV_TIMEZONE` (`America/New_York`), `TARGET_DATE` (default: today in
tz), `DRY_RUN` (false), `POST_WINDOWS` (6; how many windows to split the daily
batch across, `1` = whole batch in one run). Add new options in `config.rs`, not
by reading `std::env` elsewhere.

## How to develop and test

```bash
cargo build                       # must stay green
cargo test                        # unit tests for the pure functions
cargo clippy                      # keep lint-clean
DRY_RUN=true BSKY_HANDLE=x BSKY_APP_PASSWORD=x cargo run   # fetch+filter+compose, posts nothing
TARGET_DATE=2025-06-30 DRY_RUN=true BSKY_HANDLE=x BSKY_APP_PASSWORD=x cargo run   # backfill a specific day
```

(`BSKY_HANDLE`/`BSKY_APP_PASSWORD` are required by config even in dry-run; any
placeholder works since dry-run never logs in.)

- Prefer `DRY_RUN=true` over real posts when verifying changes.
- Unit-test the pure functions with hand-built `ArxivPaper` values / sample feed
  XML (`parse_feed(...)`); don't depend on the live feed in tests.
- **Sandbox/CI note:** some environments restrict network egress. The live feed
  (`rss.arxiv.org`) and Bluesky (`bsky.social`) may be unreachable there — test
  the pure logic offline and rely on `DRY_RUN` / `workflow_dispatch` for
  end-to-end checks where the network is open.

## Safe extension points (where future work usually lands)

- **Keyword/author filter:** extend `filter_papers` in `arxiv.rs`. It already
  receives the full `ArxivPaper`, so filter on `title`/`abstract_text`/`authors`.
- **Card thumbnail:** upload a blob via the agent in `bluesky.rs` and set the
  external embed's `thumb` (carry it through `ExternalEmbed` in `format.rs`).
- **Include cross-lists:** add `cross` to `ARXIV_ANNOUNCE_TYPES` (no code change).
- **Rich text in post body (hashtags, in-text link):** build `facets` on the
  `RecordData` in `bluesky.rs` (atrium's `RichText`/facet detection) before
  posting; re-check the grapheme budget afterward.

## Things NOT to do

- Don't add a database/cursor "to be safe" — it's redundant given the invariant
  and changes the operational model. Only add persistence if explicitly asked.
- Don't move config reads out of `config.rs`.
- Don't switch grapheme counting to `.len()`, `.chars().count()`, or a regex.
- Don't commit `.env` or any credential; secrets come from GitHub Actions
  Secrets/Variables.
- Don't assume the feed contains older papers — it only ever holds the current
  batch. Backfilling a past day is not possible from RSS alone (would need the
  arXiv API).
