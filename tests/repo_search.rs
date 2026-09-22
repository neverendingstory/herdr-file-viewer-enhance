mod common;

use common::TempDir;
use herdr_file_viewer::repo_search::{self, SearchHit};
use std::fs;

#[test]
fn returns_one_hit_per_matching_line_with_source_coordinates() {
    let tmp = TempDir::new();
    fs::create_dir_all(tmp.path().join("src")).unwrap();
    fs::write(
        tmp.path().join("src/a.rs"),
        "zero\nlet Needle = true; needle\nlast needle\n",
    )
    .unwrap();

    let output = repo_search::search(tmp.path(), "needle", false);

    assert_eq!(
        output.hits,
        vec![
            SearchHit {
                path: "src/a.rs".into(),
                line: 2,
                column: 5,
                excerpt: "let Needle = true; needle".into(),
            },
            SearchHit {
                path: "src/a.rs".into(),
                line: 3,
                column: 6,
                excerpt: "last needle".into(),
            },
        ]
    );
    assert!(!output.limited);
}

#[test]
fn uppercase_query_is_case_sensitive() {
    let tmp = TempDir::new();
    fs::write(tmp.path().join("a.txt"), "Needle\nneedle\nNEEDLE\n").unwrap();

    let output = repo_search::search(tmp.path(), "Needle", false);

    assert_eq!(output.hits.len(), 1);
    assert_eq!(output.hits[0].line, 1);
}

#[test]
fn ignored_files_follow_the_include_ignored_scope() {
    let tmp = TempDir::new();
    fs::write(tmp.path().join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(tmp.path().join("visible.txt"), "needle\n").unwrap();
    fs::write(tmp.path().join("ignored.txt"), "needle\n").unwrap();

    let project = repo_search::search(tmp.path(), "needle", false);
    assert_eq!(
        project
            .hits
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        vec!["visible.txt"]
    );

    let all = repo_search::search(tmp.path(), "needle", true);
    assert_eq!(
        all.hits
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        vec!["ignored.txt", "visible.txt"]
    );
}

#[test]
fn skips_git_binary_invalid_utf8_and_oversized_files() {
    let tmp = TempDir::new();
    fs::create_dir(tmp.path().join(".git")).unwrap();
    fs::write(tmp.path().join(".git/internal"), "needle").unwrap();
    fs::write(tmp.path().join("binary.bin"), b"needle\0tail").unwrap();
    fs::write(tmp.path().join("invalid.bin"), [0xff, 0xfe, b'n']).unwrap();
    let mut oversized = vec![b'x'; repo_search::MAX_FILE_BYTES as usize + 1];
    oversized[..6].copy_from_slice(b"needle");
    fs::write(tmp.path().join("large.txt"), oversized).unwrap();
    fs::write(tmp.path().join("ok.txt"), "needle").unwrap();

    let output = repo_search::search(tmp.path(), "needle", true);

    assert_eq!(output.hits.len(), 1);
    assert_eq!(output.hits[0].path, "ok.txt");
}

#[test]
fn caps_results_and_reports_that_more_matches_exist() {
    let tmp = TempDir::new();
    let body = (0..=repo_search::MAX_RESULTS)
        .map(|n| format!("needle {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(tmp.path().join("many.txt"), body).unwrap();

    let output = repo_search::search(tmp.path(), "needle", false);

    assert_eq!(output.hits.len(), repo_search::MAX_RESULTS);
    assert!(output.limited);
}

#[test]
fn long_excerpt_keeps_the_match_and_valid_utf8() {
    let tmp = TempDir::new();
    let prefix = "界".repeat(repo_search::MAX_EXCERPT_CHARS);
    fs::write(
        tmp.path().join("unicode.txt"),
        format!("{prefix} needle trailing text"),
    )
    .unwrap();

    let output = repo_search::search(tmp.path(), "needle", false);

    assert_eq!(output.hits.len(), 1);
    assert!(output.hits[0].excerpt.contains("needle"));
    assert!(output.hits[0].excerpt.starts_with('…'));
    assert!(output.hits[0].excerpt.chars().count() <= repo_search::MAX_EXCERPT_CHARS + 2);
}
