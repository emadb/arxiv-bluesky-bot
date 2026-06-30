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

/// A paper is treated as "published" once it carries a DOI or a journal
/// reference — i.e. a peer-reviewed version exists. Surfaced as a badge.
fn is_published(paper: &ArxivPaper) -> bool {
    paper.doi.is_some() || paper.journal_reference.is_some()
}

/// Build the trailing hashtag line (`#arXiv` plus one tag per category, which
/// doubles as the paper's cross-list) together with the byte spans each tag
/// occupies *within the line*, so callers can turn them into rich-text facets.
fn build_tags(categories: &[String]) -> (String, Vec<(usize, usize, String)>) {
    let tags = std::iter::once("arXiv".to_string()).chain(categories.iter().cloned());
    let mut line = String::new();
    let mut spans = Vec::new();
    for (i, tag) in tags.enumerate() {
        if i > 0 {
            line.push(' ');
        }
        let start = line.len();
        line.push('#');
        line.push_str(&tag);
        spans.push((start, line.len(), tag));
    }
    (line, spans)
}

/// External embed card fields, kept independent of the AT Protocol types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEmbed {
    pub uri: String,
    pub title: String,
    pub description: String,
}

/// A clickable hashtag, as a UTF-8 byte range over [`ComposedPost::text`] plus
/// the tag value (without the leading `#`). Independent of the AT Protocol
/// types; turned into a `app.bsky.richtext.facet#tag` in `bluesky.rs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagFacet {
    pub byte_start: usize,
    pub byte_end: usize,
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedPost {
    pub text: String,
    pub facets: Vec<TagFacet>,
    pub embed: ExternalEmbed,
}

/// The clickable link lives in the external embed "card", so the post text
/// itself stays short: an optional 📌 "published" badge, the title, the
/// shortened authors, and a hashtag line (`#arXiv` + the cross-list). The
/// title absorbs whatever budget the other parts leave, keeping the whole post
/// within 300 graphemes.
pub fn compose_post(paper: &ArxivPaper) -> ComposedPost {
    let badge = if is_published(paper) { "📌 " } else { "" };
    let authors_line = shorten_authors(&paper.authors);
    let (tag_line, tag_spans) = build_tags(&paper.categories);

    let block = |s: &str| grapheme_count("\n\n") + grapheme_count(s);
    let reserved = grapheme_count(badge)
        + if authors_line.is_empty() {
            0
        } else {
            block(&authors_line)
        }
        + block(&tag_line);
    let title_budget = MAX_GRAPHEMES.saturating_sub(reserved).max(20);

    let title_source = if paper.title.is_empty() {
        paper.id.as_str()
    } else {
        paper.title.as_str()
    };
    let title = truncate_graphemes(title_source, title_budget);

    let mut text = format!("{badge}{title}");
    if !authors_line.is_empty() {
        text.push_str("\n\n");
        text.push_str(&authors_line);
    }
    text.push_str("\n\n");
    // The tag line starts here; facet byte offsets are absolute into `text`.
    let tag_base = text.len();
    text.push_str(&tag_line);
    let facets = tag_spans
        .into_iter()
        .map(|(start, end, tag)| TagFacet {
            byte_start: tag_base + start,
            byte_end: tag_base + end,
            tag,
        })
        .collect();

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
        facets,
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
            categories: vec!["cs.AI".to_string(), "cs.LG".to_string()],
            journal_reference: None,
            doi: None,
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
        assert!(post.text.contains("Alice et al."));
        // The hashtag line is the last thing in the post.
        assert!(post.text.ends_with("#arXiv #cs.AI #cs.LG"));
        assert_eq!(post.embed.uri, "https://arxiv.org/abs/2506.01250");
    }

    #[test]
    fn compose_falls_back_to_id_when_empty() {
        let mut p = paper_with("", "");
        p.abstract_text = String::new();
        let post = compose_post(&p);
        assert_eq!(post.text, "2506.01250\n\n#arXiv #cs.AI #cs.LG");
        assert_eq!(post.embed.title, "arXiv:2506.01250");
        assert_eq!(post.embed.description, "arXiv:2506.01250");
    }

    #[test]
    fn hashtag_facets_cover_the_right_bytes() {
        let post = compose_post(&paper_with("A Title", "Alice"));
        // #arXiv plus one tag per category.
        let tags: Vec<&str> = post.facets.iter().map(|f| f.tag.as_str()).collect();
        assert_eq!(tags, vec!["arXiv", "cs.AI", "cs.LG"]);
        // Each facet's byte range slices out exactly "#<tag>" from the text.
        for f in &post.facets {
            assert_eq!(&post.text[f.byte_start..f.byte_end], format!("#{}", f.tag));
        }
    }

    #[test]
    fn published_paper_gets_a_badge() {
        let mut p = paper_with("A Title", "Alice");
        p.doi = Some("10.1/x".to_string());
        let post = compose_post(&p);
        assert!(post.text.starts_with("📌 A Title"));

        // A journal reference alone is enough, too.
        let mut p2 = paper_with("A Title", "Alice");
        p2.journal_reference = Some("J. Foo 2025".to_string());
        assert!(compose_post(&p2).text.starts_with("📌 "));
    }

    #[test]
    fn unpublished_paper_has_no_badge() {
        let post = compose_post(&paper_with("A Title", "Alice"));
        assert!(post.text.starts_with("A Title"));
        assert!(!post.text.contains('📌'));
    }

    #[test]
    fn badge_offset_keeps_facets_valid() {
        // With the 📌 prefix (a 4-byte emoji) the tag line shifts; facet byte
        // offsets must still land on the hashtags.
        let mut p = paper_with("A Title", "Alice");
        p.doi = Some("10.1/x".to_string());
        let post = compose_post(&p);
        for f in &post.facets {
            assert_eq!(&post.text[f.byte_start..f.byte_end], format!("#{}", f.tag));
        }
    }
}
