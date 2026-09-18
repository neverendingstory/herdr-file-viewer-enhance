mod common;

use common::TempDir;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use herdr_file_viewer::controller::{
    Components, ContentProvider, Controller, EditorHandoff, EditorOutcome, GitService,
    RenderResult, RootProviders,
};
use herdr_file_viewer::git::{Baseline, Status};
use herdr_file_viewer::intent::Intent;
use herdr_file_viewer::repo_search::SearchHit;
use herdr_file_viewer::view_policy::ViewMode;
use ratatui::text::Text;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
struct StubGit;

impl GitService for StubGit {
    fn status(&self) -> BTreeMap<PathBuf, Status> {
        BTreeMap::new()
    }

    fn changed_set(&self, _baseline: Baseline) -> BTreeMap<PathBuf, Status> {
        BTreeMap::new()
    }

    fn diff(&self, _rel_path: &Path, _baseline: Baseline, _full_context: bool) -> String {
        String::new()
    }

    fn diff_directory(&self, _rel_dir: &Path, _baseline: Baseline) -> String {
        String::new()
    }
}

#[derive(Clone, Copy)]
struct FileContent;

impl ContentProvider for FileContent {
    fn render(&self, path: &Path, _mode: ViewMode, _raw_diff: Option<&str>) -> RenderResult {
        RenderResult {
            content: Text::raw(std::fs::read_to_string(path).unwrap()),
            notices: Vec::new(),
            source: None,
        }
    }
}

struct NoopEditor;

impl EditorHandoff for NoopEditor {
    fn open(&mut self, _file: &Path) -> EditorOutcome {
        EditorOutcome::NoTakeover
    }
}

fn controller(root: &Path) -> Controller {
    let components = Components {
        providers: Box::new(|_resolved| RootProviders {
            git: Arc::new(StubGit),
            content: Box::new(FileContent),
        }),
        editor: Box::new(NoopEditor),
        clipboard: Box::new(common::RecordingClipboard::default()),
        renderers: None,
    };
    Controller::new(
        common::resolved(root.to_path_buf(), false),
        Baseline::Head,
        components,
    )
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn type_query(controller: &mut Controller, query: &str) {
    for c in query.chars() {
        controller.handle_project_search_key(key(KeyCode::Char(c)));
    }
}

fn await_hits(controller: &mut Controller) -> Vec<SearchHit> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        controller.poll();
        if let Some(hits) = controller.project_search_hits()
            && !hits.is_empty()
        {
            return hits.to_vec();
        }
        assert!(Instant::now() < deadline, "project search did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn search_scope_follows_the_current_i_state() {
    let tmp = TempDir::new();
    std::fs::write(tmp.path().join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(tmp.path().join("visible.txt"), "scope needle\n").unwrap();
    std::fs::write(tmp.path().join("ignored.txt"), "scope needle\n").unwrap();
    let mut controller = controller(tmp.path());

    controller.handle(Intent::OpenProjectSearch);
    type_query(&mut controller, "scope needle");
    let project_hits = await_hits(&mut controller);
    assert_eq!(
        project_hits
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        vec!["visible.txt"]
    );

    controller.handle_project_search_key(key(KeyCode::Esc));
    controller.handle(Intent::ToggleIgnore);
    controller.handle(Intent::OpenProjectSearch);
    type_query(&mut controller, "scope needle");
    let all_hits = await_hits(&mut controller);
    assert_eq!(
        all_hits
            .iter()
            .map(|hit| hit.path.as_str())
            .collect::<Vec<_>>(),
        vec!["ignored.txt", "visible.txt"]
    );
}

#[test]
fn enter_opens_the_selected_result_in_source_view_at_its_line() {
    let tmp = TempDir::new();
    std::fs::write(
        tmp.path().join("target.md"),
        "line one\nline two\nline three\nunique needle\nline five\nline six\n",
    )
    .unwrap();
    let mut controller = controller(tmp.path());
    controller.set_content_viewport(40, 2);

    controller.handle(Intent::OpenProjectSearch);
    type_query(&mut controller, "unique needle");
    let hits = await_hits(&mut controller);
    assert_eq!((hits[0].path.as_str(), hits[0].line), ("target.md", 4));

    controller.handle_project_search_key(key(KeyCode::Enter));
    let deadline = Instant::now() + Duration::from_secs(5);
    while controller.content_scroll() != 3 {
        controller.poll();
        assert!(Instant::now() < deadline, "source-line jump did not apply");
        std::thread::sleep(Duration::from_millis(5));
    }

    assert!(!controller.project_search_open());
    assert_eq!(
        controller.selected_view_mode(),
        Some(ViewMode::SyntaxContent)
    );
    let selected = controller
        .tree()
        .selected()
        .expect("search result selected");
    assert_eq!(
        selected.path.file_name().and_then(|name| name.to_str()),
        Some("target.md")
    );
}
