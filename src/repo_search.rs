//! Bounded, read-only project content search.
//!
//! The scanner walks the viewer root with the same Git-ignore policy as the file index, reads only
//! bounded UTF-8 text files, and returns one row per matching source line. It performs no writes and
//! degrades by skipping files that disappear, cannot be read, look binary, or exceed the size cap.

use crate::{index, search};
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Files larger than this are skipped rather than read into memory.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Maximum number of matching-line rows retained for one query.
pub const MAX_RESULTS: usize = 500;
/// Maximum excerpt body width in Unicode scalar values, excluding edge ellipses.
pub const MAX_EXCERPT_CHARS: usize = 160;

/// One matching source line under the current viewer root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// Root-relative, forward-slash path.
    pub path: String,
    /// One-based source line.
    pub line: usize,
    /// One-based Unicode-scalar column of the first occurrence on the line.
    pub column: usize,
    /// Bounded source-line context containing the first occurrence.
    pub excerpt: String,
}

/// Complete bounded output for one query.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchOutput {
    pub hits: Vec<SearchHit>,
    /// `true` when at least one additional matching line existed beyond [`MAX_RESULTS`].
    pub limited: bool,
}

/// Search text files under `root` using literal smartcase matching.
///
/// `include_ignored` mirrors the tree's `i` state. The `.git` subtree is excluded in both modes by
/// [`index::build_with_ignored`]. Results are deterministic because that index is path-sorted and
/// lines are visited in source order.
pub fn search(root: &Path, query: &str, include_ignored: bool) -> SearchOutput {
    if query.is_empty() {
        return SearchOutput::default();
    }

    let mut output = SearchOutput::default();
    for relative in index::build_with_ignored(root, include_ignored) {
        let Some(text) = read_bounded_text(&root.join(&relative)) else {
            continue;
        };
        for (line_index, line) in text.lines().enumerate() {
            let Some((start, end)) = search::first_match(query, line) else {
                continue;
            };
            if output.hits.len() == MAX_RESULTS {
                output.limited = true;
                return output;
            }
            output.hits.push(SearchHit {
                path: relative.clone(),
                line: line_index + 1,
                column: line[..start].chars().count() + 1,
                excerpt: excerpt_around(line, start, end),
            });
        }
    }
    output
}

fn read_bounded_text(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FILE_BYTES || bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn excerpt_around(line: &str, match_start: usize, match_end: usize) -> String {
    let total = line.chars().count();
    if total <= MAX_EXCERPT_CHARS {
        return line.trim().to_string();
    }

    let match_start_char = line[..match_start].chars().count();
    let match_len = line[match_start..match_end].chars().count();
    let before_budget = MAX_EXCERPT_CHARS.saturating_sub(match_len) / 2;
    let mut start = match_start_char.saturating_sub(before_budget);
    let mut end = (start + MAX_EXCERPT_CHARS).min(total);
    if end == total {
        start = end.saturating_sub(MAX_EXCERPT_CHARS);
    }
    // A query can itself exceed the normal excerpt budget. Keep the complete match visible rather
    // than returning a row whose context does not contain the text that produced it.
    if match_start_char + match_len > end {
        end = (match_start_char + match_len).min(total);
        start = end.saturating_sub(MAX_EXCERPT_CHARS.max(match_len));
    }

    let body: String = line.chars().skip(start).take(end - start).collect();
    let mut excerpt = String::new();
    if start > 0 {
        excerpt.push('…');
    }
    excerpt.push_str(body.trim());
    if end < total {
        excerpt.push('…');
    }
    excerpt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_keeps_a_long_match_whole() {
        let query = "needle".repeat(40);
        let line = format!("prefix {query} suffix");
        let start = "prefix ".len();
        let excerpt = excerpt_around(&line, start, start + query.len());
        assert!(excerpt.contains(&query));
    }
}
