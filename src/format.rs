//! Compose a Bluesky post for a paper, staying within the 300-grapheme limit.
//!
//! The clickable link/title/abstract live in an external embed "card" (here a
//! plain [`ExternalEmbed`], turned into the AT Protocol type in `bluesky.rs`)
//! so the post text itself stays short.

use crate::arxiv::ArxivPaper;
use unicode_segmentation::UnicodeSegmentation;

const MAX_GRAPHEMES: usize = 300;

fn grapheme_count(s: &str) -> usize {
    s.graphemes(true).count()
}

/// Truncate to at most `max` graphemes, appending an ellipsis if cut.
fn truncate_graphemes(s: &str, max: usize) -> String {
    if grapheme_count(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for (n, g) in s.graphemes(true).enumerate() {
        if n >= max.saturating_sub(1) {
            break;
        }
        out.push_str(g);
    }
    out.push('\u{2026}');
    out
}

/// arXiv author lists can be enormous; keep posts readable.
fn shorten_authors(authors: &str) -> String {
    let list: Vec<&str> = authors
        .split(',')
        .map(|a| a.trim())
        .filter(|a| !a.is_empty())
        .collect();
    match list.len() {
        0 => String::new(),
        1..=3 => list.join(", "),
        _ => format!("{} et al.", list[0]),
    }
}

/// External embed card fields, kept independent of the AT Protocol types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEmbed {
    pub uri: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedPost {
    pub text: String,
    pub embed: ExternalEmbed,
}

/// The clickable link lives in the external embed "card", so the post text
/// itself stays short: title + authors, trimmed to fit 300 graphemes.
pub fn compose_post(paper: &ArxivPaper) -> ComposedPost {
    let authors_line = shorten_authors(&paper.authors);
    let separator = if authors_line.is_empty() { "" } else { "\n\n" };
    let reserved = grapheme_count(separator) + grapheme_count(&authors_line);
    let title_budget = MAX_GRAPHEMES.saturating_sub(reserved).max(20);

    let title_source = if paper.title.is_empty() {
        paper.id.as_str()
    } else {
        paper.title.as_str()
    };
    let title = truncate_graphemes(title_source, title_budget);
    let text = format!("{title}{separator}{authors_line}");

    let card_title = if paper.title.is_empty() {
        format!("arXiv:{}", paper.id)
    } else {
        paper.title.clone()
    };
    let card_description = if paper.abstract_text.is_empty() {
        format!("arXiv:{}", paper.id)
    } else {
        paper.abstract_text.clone()
    };

    ComposedPost {
        text,
        embed: ExternalEmbed {
            uri: paper.link.clone(),
            title: truncate_graphemes(&card_title, MAX_GRAPHEMES),
            description: truncate_graphemes(&card_description, MAX_GRAPHEMES),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paper_with(title: &str, authors: &str) -> ArxivPaper {
        ArxivPaper {
            id: "2506.01250".to_string(),
            title: title.to_string(),
            authors: authors.to_string(),
            abstract_text: "An abstract.".to_string(),
            link: "https://arxiv.org/abs/2506.01250".to_string(),
            announce_type: "new".to_string(),
            announce_date: "2025-06-30".to_string(),
            categories: vec![],
        }
    }

    #[test]
    fn grapheme_count_counts_clusters_not_code_units() {
        // Family emoji (ZWJ sequence) + flag are single graphemes each.
        assert_eq!(grapheme_count("👨‍👩‍👧‍👦"), 1);
        assert_eq!(grapheme_count("🇮🇹"), 1);
        assert_eq!(grapheme_count("é"), 1); // combining acute
        assert_eq!(grapheme_count("abc"), 3);
    }

    #[test]
    fn truncate_appends_ellipsis_when_cut() {
        let out = truncate_graphemes("abcdef", 4);
        assert_eq!(out, "abc\u{2026}");
        assert_eq!(grapheme_count(&out), 4);
        assert_eq!(truncate_graphemes("abc", 4), "abc");
    }

    #[test]
    fn shorten_authors_collapses_long_lists() {
        assert_eq!(shorten_authors(""), "");
        assert_eq!(shorten_authors("Alice, Bob"), "Alice, Bob");
        assert_eq!(shorten_authors("A, B, C"), "A, B, C");
        assert_eq!(shorten_authors("A, B, C, D"), "A et al.");
    }

    #[test]
    fn compose_keeps_text_within_limit() {
        let long_title = "word ".repeat(200);
        let post = compose_post(&paper_with(long_title.trim(), "Alice, Bob, Carol, Dave"));
        assert!(grapheme_count(&post.text) <= MAX_GRAPHEMES);
        assert!(post.text.ends_with("Alice et al."));
        assert_eq!(post.embed.uri, "https://arxiv.org/abs/2506.01250");
    }

    #[test]
    fn compose_falls_back_to_id_when_empty() {
        let mut p = paper_with("", "");
        p.abstract_text = String::new();
        let post = compose_post(&p);
        assert_eq!(post.text, "2506.01250");
        assert_eq!(post.embed.title, "arXiv:2506.01250");
        assert_eq!(post.embed.description, "arXiv:2506.01250");
    }
}
