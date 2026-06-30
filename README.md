# arxiv-bluesky-bot

Posts each day's **newly announced** arXiv papers to a Bluesky account, once a
day, on a GitHub Actions schedule. **No database, no stored state.**

## How it avoids duplicates without a database

arXiv's RSS feeds have two properties that make state unnecessary:

1. The feed is **rebuilt daily at 00:00 US/Eastern** and only contains the
   latest announcement batch.
2. Every item is stamped with an **announcement date** (`pubDate`) and an
   **`arxiv:announce_type`** (`new`, `cross`, `replace`, `replace-cross`).
3. The feed is **empty on Saturdays, Sundays, and arXiv holidays**.

So the bot simply fetches the feed and keeps the items whose announcement date
equals **today** (in US/Eastern) and whose type is one you want (default:
`new`). The GitHub job runs at 09:00 UTC — a few hours after the 00:00 Eastern
refresh — so "today in Eastern" is exactly the date on the fresh batch. Run it
once a day and each batch is posted exactly once; weekend/holiday runs match
nothing and post nothing.

> Note on wording: the batch announced on day *D* is made up of papers
> **submitted the day before**, so "today's batch" and "yesterday's papers" are
> the same set. If you'd rather pin an explicit day, set `TARGET_DATE`.

## Setup

1. **Create a Bluesky account** for the bot and generate an **App Password**
   (Settings → Privacy and Security → App Passwords). Never use your real
   password.

2. **Fork/clone this repo** into your own GitHub account.

3. **Add the secrets** (Settings → Secrets and variables → Actions → *Secrets*):
   - `BSKY_HANDLE` — e.g. `arxiv-cs.bsky.social`
   - `BSKY_APP_PASSWORD` — the app password from step 1

4. **(Optional) add Variables** (same screen → *Variables*) to override defaults:
   | Variable | Default | Meaning |
   |---|---|---|
   | `ARXIV_CATEGORIES` | `cs.AI` | One feed path; combine with `+` e.g. `cs.AI+cs.LG` |
   | `ARXIV_ANNOUNCE_TYPES` | `new` | Comma list: `new,cross,replace,replace-cross` |
   | `MAX_POSTS` | `50` | Hard cap per run (busy categories produce many papers/day) |
   | `POST_DELAY_MS` | `1500` | Pause between posts |

5. Done. The workflow runs daily at 09:00 UTC. To test first, use
   **Actions → Post arXiv to Bluesky → Run workflow** with **dry_run = true**.

## Run locally

```bash
cp .env.example .env        # fill in handle + app password
set -a && source .env && set +a
DRY_RUN=true cargo run      # see what it would post
cargo run                   # actually post
```

Backfill a specific day (`POST_WINDOWS=1` posts the whole batch at once):

```bash
TARGET_DATE=2025-06-30 POST_WINDOWS=1 cargo run
```

## Choosing categories

Browse the full taxonomy at <https://arxiv.org/category_taxonomy>. The feed
path is the part after `https://rss.arxiv.org/rss/` — a whole archive (`cs`),
a subject class (`cs.AI`), or several joined with `+`.

## Notes & limits

- **Volume.** A single busy class (`cs.LG`, `cs.AI`) can be dozens to hundreds
  of papers per weekday. `MAX_POSTS` caps a run; narrow the category or add a
  keyword filter in `src/arxiv.rs` (`filter_papers`) if that's too much.
- **Post shape.** Text is an optional `📌` badge (shown when the paper already
  has a DOI / journal reference, i.e. a published version), the `title`,
  shortened authors, and a hashtag line — `#arXiv` plus one clickable tag per
  category (which doubles as the paper's cross-list). The link, title and
  abstract ride in a Bluesky link card. The 300-grapheme post limit is enforced
  by truncation in `src/format.rs`.
- **No thumbnail** is attached to the card (keeps it simple). You can add one by
  uploading a blob via the agent and setting the external embed's `thumb`.
- The bot exits non-zero if any individual post fails, so failures show up red
  in the Actions log, but it still attempts every paper in the batch.

## Layout

```
src/
  config.rs    env → typed config
  arxiv.rs     fetch + parse + filter the RSS feed; window slicing
  format.rs    compose post text + link card (300-grapheme safe)
  bluesky.rs   login + post via bsky-sdk (AT Protocol)
  main.rs      orchestration
.github/workflows/post.yml   the daily schedule
```
