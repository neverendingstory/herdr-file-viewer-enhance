//! File Index — a recursive walk that returns every file under `root` as a root-relative path
//! string, with an optional Git-ignore scope.
//!
//! Shared by Go-to-file and project-content search. This is a separate walk from the Tree Model
//! (ADR-0005): no depth limit, files only, and the entire `.git` subtree is pruned via
//! `filter_entry`.

use ignore::WalkBuilder;
use std::path::Path;

/// The shared base for the crate's two gitignore-aware walks — this File Index and the Tree
/// Model (`tree.rs`). Sets the hermetic policy both share so it lives in one place: honor an
/// ancestor `.gitignore`, ignore the user's global gitignore and generic `.ignore` files, and
/// apply `.gitignore` even outside a git repo. The caller sets what differs between the two
/// walks — depth, dotfile hiding, and whether `.gitignore`/`.git/info/exclude` are honored.
pub(crate) fn walk_builder(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .parents(true) // honor ancestor .gitignore for correct nested semantics
        .git_global(false) // hermetic: ignore the user's global gitignore
        .ignore(false) // only git ignore sources, not generic .ignore files
        .require_git(false); // honor .gitignore even outside a git repo (AC-13, AC-19, AC-4, AC-26)
    builder
}

/// Return every file under `root` as a root-relative `String`, respecting `.gitignore`.
///
/// - Recursive (no depth limit) — AC-12.
/// - `.gitignore`-d files are excluded — AC-13.
/// - The `.git` subtree is pruned entirely — AC-14.
/// - Directories are not included, only files — AC-15.
/// - Every returned path is relative to `root` (no leading `/`, no `..`) — AC-N5.
/// - Each call performs a fresh walk; no cache — AC-18.
/// - Works in non-git directories without error (`require_git(false)`) — AC-19.
/// - Read-only: no filesystem or git mutations — AC-N1, AC-N2.
pub fn build(root: &Path) -> Vec<String> {
    build_with_ignored(root, false)
}

/// Return every file under `root`, optionally including files matched by Git ignore sources.
///
/// `include_ignored` mirrors the tree's `i` toggle. The `.git` subtree is always pruned, even
/// when ignored files are included. Results are sorted so consumers such as project search have a
/// deterministic traversal order across platforms.
pub fn build_with_ignored(root: &Path, include_ignored: bool) -> Vec<String> {
    let mut builder = walk_builder(root);
    builder
        .hidden(false) // include dotfiles (AC-17 depends on the index NOT hiding dotfiles)
        .git_ignore(!include_ignored)
        .git_exclude(!include_ignored)
        .filter_entry(|e| e.file_name() != ".git"); // prune entire .git subtree — AC-14

    let mut paths: Vec<String> = builder
        .build()
        .filter_map(Result::ok) // skip unreadable entries; traversal continues
        .filter(|e| e.file_type().is_some_and(|t| t.is_file())) // files only — AC-15
        .filter_map(|e| e.path().strip_prefix(root).ok().map(rel_to_slash))
        .collect();
    paths.sort();
    paths
}

/// Render a root-relative path as a forward-slash string on every platform. The rest of the app
/// (git status/diff/worktree paths, the tree, the content title) speaks git's forward-slash
/// convention; on Windows the native separator is `\`, so a raw stringification would make the
/// finder's listing inconsistent with the rest of the UI. Joining the path's `Normal` components
/// with `/` is identical to today's output on unix (the separator already is `/`) and converts
/// `a\b` → `a/b` on Windows. It also enforces AC-N5 (root-relative, no `..`/absolute leak) by
/// construction.
fn rel_to_slash(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}
