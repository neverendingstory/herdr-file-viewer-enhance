# Project Content Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an `s`-opened, keyboard-first project-content search that lists `path:line` excerpts, follows the current `i` / `show_ignored` scope, and opens the selected match at its source line.

**Architecture:** Add a bounded repository scanner over the existing `ignore` walk and run it on a dedicated background worker so input and drawing never block. Add a mutually exclusive `ProjectSearch` modal with debounced, sequence-guarded incremental requests; project its results through the existing centered finder overlay, and reuse `Controller::apply_open_target` for the proven reveal → source-view → deferred line-jump chain. The scanner skips `.git`, non-UTF-8/NUL-bearing files, unreadable files, and files larger than 1 MiB; it returns at most 500 deterministic matching-line rows with smartcase literal matching.

**Tech Stack:** Rust 1.96 / edition 2024, `ignore` 0.4, `std::thread` + `mpsc`, crossterm key events, ratatui presenter, existing controller/open-target seams.

**Repository constraint:** The commit commands below are checkpoints only. Do not run them unless the user explicitly asks for commits.

---

## File map

**Create**

- `src/repo_search.rs` — bounded filesystem content scanner and typed search output.
- `src/project_search.rs` — pure modal state: prompt, results, cursor, horizontal scroll, and loading/limit flags.
- `src/controller/project_search.rs` — controller projection, request dispatch, raw-key handling, and confirmation.
- `tests/repo_search.rs` — deterministic scanner coverage for smartcase, ignored scope, binary/size guards, and result shape.
- `tests/project_search.rs` — controller-level `s` → results → Enter → source-line flow.

**Modify**

- `src/index.rs` — parameterize the existing walk so the scanner can include ignored files when `i` is on while `.git` remains pruned.
- `src/search.rs` — expose one-line smartcase matching so in-file and project search cannot drift semantically.
- `src/lib.rs` — export the two new focused modules.
- `src/intent.rs` — add `OpenProjectSearch` and keep the read-only exhaustive matrix complete.
- `src/input.rs` — register remappable `project_search = "s"` under Search & jump.
- `src/controller/mod.rs` — add modal variant, worker channels/sequences, polling, intent route, and view projection wiring.
- `src/controller/mouse.rs` — make the new keyboard-only modal swallow mouse events instead of leaking them to the underlying columns.
- `src/app.rs` — route raw keys to the project-search modal before global decoding.
- `src/presenter.rs` — let the existing finder popup render either file-path or content-search rows/status without changing the old `f` UI.
- `tests/index.rs` — prove the optional ignored-file scope and permanent `.git` exclusion.
- `tests/presenter.rs` — cover the content-search title, loading/no-match state, `path:line` rows, and unchanged file-finder snapshots.
- `tests/e2e_keyboard.rs` — one keyboard journey for opening, querying, confirming, and landing on a line.
- `docs/keys.md` — document `s` and fixed modal controls.
- `docs/configuration.md` — document the `project_search` remappable action.
- `docs/usage.md` — add the content-search behavior and `i` scope relationship.
- `ARCHITECTURE.md` — add the scanner/state/controller responsibilities and async data flow.
- `CHANGELOG.md` — add one Unreleased/Added bullet.

---

### Task 1: Scope-aware index and shared smartcase primitive

**Files:**
- Modify: `src/index.rs`
- Modify: `src/search.rs`
- Test: `tests/index.rs`
- Test: `tests/search.rs`

- [ ] **Step 1: Add failing index tests for both `i` scopes**

Append tests that create `.gitignore`, a normal file, an ignored file, and `.git/internal`; assert the default API excludes ignored files, the opt-in API includes them, and neither API returns `.git` paths:

```rust
#[test]
fn include_ignored_policy_mirrors_i_without_exposing_dot_git() {
    let tmp = common::TempDir::new();
    let root = tmp.path();
    fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(root.join("visible.txt"), "visible").unwrap();
    fs::write(root.join("ignored.txt"), "ignored").unwrap();
    fs::create_dir(root.join(".git")).unwrap();
    fs::write(root.join(".git/internal"), "private").unwrap();

    let project = index::build_with_ignored(root, false);
    assert!(project.iter().any(|p| p == "visible.txt"));
    assert!(!project.iter().any(|p| p == "ignored.txt"));

    let all = index::build_with_ignored(root, true);
    assert!(all.iter().any(|p| p == "ignored.txt"));
    assert!(!all.iter().any(|p| p.starts_with(".git")));
}
```

- [ ] **Step 2: Run the index test and verify the missing API failure**

Run: `cargo test --test index include_ignored_policy_mirrors_i_without_exposing_dot_git`

Expected: compile failure because `index::build_with_ignored` does not exist.

- [ ] **Step 3: Parameterize the existing index walk without changing `f`**

Keep `build(root)` as the compatibility/default entry point and add:

```rust
pub fn build(root: &Path) -> Vec<String> {
    build_with_ignored(root, false)
}

pub fn build_with_ignored(root: &Path, include_ignored: bool) -> Vec<String> {
    let mut builder = walk_builder(root);
    builder
        .hidden(false)
        .git_ignore(!include_ignored)
        .git_exclude(!include_ignored)
        .filter_entry(|e| e.file_name() != ".git");

    let mut paths: Vec<String> = builder
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| e.path().strip_prefix(root).ok().map(rel_to_slash))
        .collect();
    paths.sort();
    paths
}
```

Sorting makes project-search output deterministic and also stabilizes the current `f` candidate order before fuzzy ranking.

- [ ] **Step 4: Add failing tests for a shared first-match helper**

Add unit coverage in `tests/search.rs` for lowercase case-insensitive matching, uppercase case-sensitive matching, UTF-8-safe byte offsets, and empty query:

```rust
#[test]
fn first_match_uses_the_same_smartcase_rule() {
    assert_eq!(search::first_match("needle", "x NEEDLE y"), Some((2, 8)));
    assert_eq!(search::first_match("Needle", "x NEEDLE y"), None);
    assert_eq!(search::first_match("é", "aéz"), Some((1, 3)));
    assert_eq!(search::first_match("", "anything"), None);
}
```

- [ ] **Step 5: Run the shared-search test and verify failure**

Run: `cargo test --test search first_match_uses_the_same_smartcase_rule`

Expected: compile failure because `search::first_match` does not exist.

- [ ] **Step 6: Implement `first_match` and route `find_matches` through the same case rule**

Add this public seam in `src/search.rs`:

```rust
pub fn first_match(query: &str, line: &str) -> Option<(usize, usize)> {
    if query.is_empty() {
        return None;
    }
    let case_sensitive = query.chars().any(|c| c.is_ascii_uppercase());
    if case_sensitive {
        line.find(query).map(|start| (start, start + query.len()))
    } else {
        let needle = query.to_ascii_lowercase();
        line.to_ascii_lowercase()
            .find(&needle)
            .map(|start| (start, start + needle.len()))
    }
}
```

Keep `find_matches`’ all-occurrences behavior, but extract its smartcase/needle preparation into a private helper used by both paths so the two rules cannot diverge.

- [ ] **Step 7: Run focused tests**

Run: `cargo test --test index && cargo test --test search`

Expected: all index and search tests pass.

- [ ] **Step 8: Checkpoint commit (only with explicit user authorization)**

```bash
git add src/index.rs src/search.rs tests/index.rs tests/search.rs
git commit -m "refactor: share project search scope and matching\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 2: Bounded repository scanner

**Files:**
- Create: `src/repo_search.rs`
- Modify: `src/lib.rs`
- Test: `tests/repo_search.rs`

- [ ] **Step 1: Write scanner tests before implementation**

Create tests for:

1. one result per matching source line with 1-based line numbers;
2. smartcase literal matching;
3. `include_ignored = false/true` behavior;
4. `.git`, NUL-bearing/non-UTF-8, unreadable/vanished, and >1 MiB files being skipped without error;
5. `MAX_RESULTS` setting `limited = true` and returning exactly the cap;
6. excerpt truncation remaining valid UTF-8 and including the matched text.

Use this expected public shape:

```rust
use herdr_file_viewer::repo_search::{self, SearchHit};

let output = repo_search::search(root, "needle", false);
assert_eq!(
    output.hits,
    vec![SearchHit {
        path: "src/a.rs".into(),
        line: 2,
        column: 5,
        excerpt: "let needle = true;".into(),
    }]
);
assert!(!output.limited);
```

- [ ] **Step 2: Run and verify the module-missing failure**

Run: `cargo test --test repo_search`

Expected: compile failure because `repo_search` is not exported.

- [ ] **Step 3: Implement typed bounded scanning**

Create `src/repo_search.rs` with:

```rust
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_RESULTS: usize = 500;
pub const MAX_EXCERPT_CHARS: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub path: String,
    pub line: usize,
    pub column: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchOutput {
    pub hits: Vec<SearchHit>,
    pub limited: bool,
}

pub fn search(root: &Path, query: &str, include_ignored: bool) -> SearchOutput;
```

Implementation rules:

- return empty immediately for an empty query;
- enumerate `index::build_with_ignored(root, include_ignored)`;
- open each file and read through `Read::take(MAX_FILE_BYTES + 1)` so a metadata race cannot allocate an unbounded buffer;
- skip when read length exceeds the cap, bytes contain NUL, or `String::from_utf8` fails;
- use `search::first_match` once per line and emit at most one result per line;
- use 1-based `line`, 1-based display `column`, root-relative slash paths;
- build a Unicode-safe excerpt centered around the match, collapse surrounding leading/trailing whitespace only, and prefix/suffix `…` when context was removed;
- stop retaining rows at 500 and set `limited` when a 501st matching line is observed;
- never return filesystem errors; a file that disappears during the scan is skipped.

Export it from `src/lib.rs`:

```rust
pub mod repo_search;
```

- [ ] **Step 4: Run scanner tests**

Run: `cargo test --test repo_search`

Expected: all scanner tests pass.

- [ ] **Step 5: Run formatting and clippy for the scanner**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`

Expected: both commands exit 0.

- [ ] **Step 6: Checkpoint commit (only with explicit user authorization)**

```bash
git add src/repo_search.rs src/lib.rs tests/repo_search.rs
git commit -m "feat: add bounded repository content scanner\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 3: Project-search modal state and background worker

**Files:**
- Create: `src/project_search.rs`
- Create: `src/controller/project_search.rs`
- Modify: `src/lib.rs`
- Modify: `src/controller/mod.rs`
- Modify: `src/controller/mouse.rs`
- Test: `tests/project_search.rs`

- [ ] **Step 1: Add pure modal-state tests**

In `src/project_search.rs`, first write unit tests proving:

- editing resets cursor, hscroll, previous hits, and marks a nonempty query searching;
- empty query is idle;
- applying results clears searching and clamps the cursor;
- up/down selection clamps;
- left/right horizontal scroll saturates and clamps;
- `selected()` returns the exact typed hit.

Define this state API:

```rust
pub struct ProjectSearchState {
    prompt: PromptInput,
    hits: Vec<SearchHit>,
    cursor: usize,
    hscroll: u16,
    searching: bool,
    limited: bool,
    include_ignored: bool,
}

impl ProjectSearchState {
    pub fn new(include_ignored: bool) -> Self;
    pub fn query(&self) -> &str;
    pub fn hits(&self) -> &[SearchHit];
    pub fn cursor(&self) -> usize;
    pub fn hscroll(&self) -> u16;
    pub fn searching(&self) -> bool;
    pub fn limited(&self) -> bool;
    pub fn include_ignored(&self) -> bool;
    pub fn push(&mut self, c: char);
    pub fn backspace(&mut self);
    pub fn apply(&mut self, output: SearchOutput);
    pub fn move_selection(&mut self, delta: isize);
    pub fn selected(&self) -> Option<&SearchHit>;
    pub fn scroll_left(&mut self);
    pub fn scroll_right(&mut self);
    pub fn clamp_hscroll(&mut self, max: u16);
}
```

- [ ] **Step 2: Run unit tests and verify failure**

Run: `cargo test --lib project_search`

Expected: compile/module failure before the state exists.

- [ ] **Step 3: Implement the modal state and export it**

Implement the API above in `src/project_search.rs`, following `FinderState`’s clamping and eight-column horizontal-scroll behavior. Add:

```rust
pub mod project_search;
```

to `src/lib.rs`.

- [ ] **Step 4: Add a failing controller integration test**

Create `tests/project_search.rs` with a temp root containing a normal file and an ignored file. Build a real controller with a fixed content provider, call `handle(Intent::OpenProjectSearch)`, type a query through `handle_project_search_key`, and poll until `project_search_hits()` is nonempty. Assert:

- default scope only returns the normal file;
- after closing, toggling `i`, reopening, and querying, the ignored hit appears;
- selecting a hit and pressing Enter closes the modal, reveals that path, forces `SyntaxContent`, and after the render lands sets `content_scroll()` to the expected line.

Use a generous five-second deadline and a short polling sleep only for the positive arrival; do not use sleeping to prove negatives.

- [ ] **Step 5: Run and verify missing controller surface**

Run: `cargo test --test project_search`

Expected: compile failure for the missing intent/controller methods.

- [ ] **Step 6: Add the modal variant and dedicated worker types**

In `src/controller/mod.rs`, add:

```rust
struct ProjectSearchJob {
    seq: u64,
    root: PathBuf,
    query: String,
    include_ignored: bool,
}

struct ProjectSearchCompletion {
    seq: u64,
    output: crate::repo_search::SearchOutput,
}
```

Add `Modal::ProjectSearch(ProjectSearchState)` plus immutable/mutable accessors, and add these controller fields:

```rust
project_search_tx: mpsc::Sender<ProjectSearchJob>,
project_search_rx: mpsc::Receiver<ProjectSearchCompletion>,
project_search_seq: u64,
```

Spawn one long-lived worker in `Controller::new`. Its loop must receive one job, collapse any queued jobs to the newest with `try_recv`, run `repo_search::search` off the input thread, and send the typed completion. This matches the existing render worker’s no-`tokio`, stale-result pattern.

- [ ] **Step 7: Add controller orchestration in a focused submodule**

Create `src/controller/project_search.rs` implementing:

```rust
pub(super) fn open_project_search(&mut self) -> Effects;
pub fn project_search_open(&self) -> bool;
pub fn handle_project_search_key(&mut self, key: KeyEvent) -> Effects;
pub(super) fn project_search_view(&self) -> Option<FinderView>;
pub fn project_search_hits(&self) -> Option<&[SearchHit]>;
```

Behavior:

- opening captures `self.show_ignored` into a fresh state;
- printable chars/backspace edit the prompt, clear stale rows, increment `project_search_seq`, and send a nonempty query job immediately; empty query sends nothing but still invalidates prior completions;
- Up/Down select; Left/Right scroll; Esc closes; Enter is inert while searching or without a row;
- a valid Enter clones the hit, closes the modal, and calls:

```rust
self.apply_open_target(&crate::open_target::OpenTarget {
    path: hit.path,
    line: Some(hit.line),
    end_line: None,
});
```

- `poll()` drains `project_search_rx`, applying only the completion whose `seq == project_search_seq` while `Modal::ProjectSearch` is still open;
- re-root closes the modal; later old-root completions are stale by sequence/modal guard;
- `set_pane_geometry` clamps whichever finder-like modal is open;
- `src/controller/mouse.rs` treats `Modal::ProjectSearch(_)` as keyboard-only and returns `Effects::noop()`.

Declare the controller submodule in `src/controller/mod.rs`:

```rust
mod project_search;
```

- [ ] **Step 8: Run modal and integration tests**

Run: `cargo test --lib project_search && cargo test --test project_search`

Expected: all tests pass.

- [ ] **Step 9: Checkpoint commit (only with explicit user authorization)**

```bash
git add src/project_search.rs src/controller/project_search.rs src/controller/mod.rs src/controller/mouse.rs src/lib.rs tests/project_search.rs
git commit -m "feat: add asynchronous project search modal\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 4: Global key registration and raw-key routing

**Files:**
- Modify: `src/intent.rs`
- Modify: `src/input.rs`
- Modify: `src/controller/mod.rs`
- Modify: `src/app.rs`
- Test: existing unit tests in `src/intent.rs`, `src/input.rs`, and controller tests

- [ ] **Step 1: Add `OpenProjectSearch` to the closed intent set**

Add the variant beside `OpenFinder`:

```rust
/// Open project-wide content search over files admitted by the current `i` scope.
/// Read-only: it scans and navigates but never mutates repository data.
OpenProjectSearch,
```

Increase the current `Intent::ALL` array length by one, insert the variant once, and add it to `intent_effects_never_mutate_files_or_git_and_classify_annotation_edits` as `(false, false)`. Do not remove or weaken any existing row.

- [ ] **Step 2: Register the nonconflicting `s` key**

Add a `REGISTRY` row immediately after `open_finder`:

```rust
Binding {
    intent: Intent::OpenProjectSearch,
    name: "project_search",
    default_keys: &[KeyCode::Char('s')],
    description: "Search file contents across the current project scope.",
    category: "Search & jump",
},
```

- [ ] **Step 3: Route the intent and enforce modal exclusion**

In `Controller::handle`, guard `Modal::ProjectSearch` like the existing finder/prompt guards and add:

```rust
Intent::OpenProjectSearch => self.open_project_search(),
```

Any exhaustive policy match that fails to compile must classify the new action as read-only and available from pinned focus.

- [ ] **Step 4: Route raw keys before global key decoding**

In `src/app.rs`, add an event-loop arm before the file-finder arm:

```rust
Event::Key(key)
    if key.kind == KeyEventKind::Press && controller.project_search_open() =>
{
    let fx = controller.handle_project_search_key(key);
    // Apply clear/quit/redraw exactly as the existing finder arm does.
}
```

This ensures `j`, `q`, `s`, and other printable keys edit the query rather than trigger global actions.

- [ ] **Step 5: Run intent/input/controller tests**

Run: `cargo test --lib intent && cargo test --lib input && cargo test controller`

Expected: all tests pass; registry uniqueness/completeness confirms `s` has no collision.

- [ ] **Step 6: Checkpoint commit (only with explicit user authorization)**

```bash
git add src/intent.rs src/input.rs src/controller/mod.rs src/app.rs
git commit -m "feat: bind project content search to s\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 5: Reuse the finder popup for content results

**Files:**
- Modify: `src/presenter.rs`
- Modify: `src/controller/finder.rs`
- Modify: `src/controller/project_search.rs`
- Modify: `src/controller/mod.rs`
- Test: `tests/presenter.rs`
- Snapshot: `tests/snapshots/presenter__presenter_project_search_results.snap`

- [ ] **Step 1: Add failing presenter coverage**

Build a view whose finder-like projection represents content search and assert it renders:

- title `Search contents` plus scope (`project` or `all files`);
- placeholder `Type to search file contents` for an empty query;
- `Searching…` after a nonempty query is dispatched;
- `No matches` after a completed empty result;
- rows such as `src/app.rs:42  fn needle()`;
- `Showing first 500 matches` when limited;
- reversed style on the selected result;
- the existing two `f` snapshots unchanged.

- [ ] **Step 2: Run the presenter tests and verify failure**

Run: `cargo test --test presenter project_search`

Expected: compile/assertion failure because `FinderView` has no mode/status metadata.

- [ ] **Step 3: Generalize the draw model without duplicating popup geometry**

Add:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinderKind {
    File,
    ProjectContent {
        searching: bool,
        limited: bool,
        include_ignored: bool,
    },
}
```

and a `kind: FinderKind` field to `FinderView`. The file finder supplies `FinderKind::File`; the project search supplies `ProjectContent` and preformats each hit as:

```rust
format!("{}:{}  {}", hit.path, hit.line, hit.excerpt)
```

Update `finder_overlay_layout` and `draw_finder_overlay` to derive title, placeholder, status-only row, and footer from `kind` while continuing to use exactly one geometry calculation. For `FinderKind::File`, preserve every existing string and sizing decision so the old snapshots remain byte-for-byte unchanged.

- [ ] **Step 4: Merge the two projections into `ViewState.finder`**

In `Controller::view_state`, use:

```rust
finder: self.project_search_view().or_else(|| self.finder_view()),
```

The `Modal` enum guarantees both cannot exist simultaneously.

- [ ] **Step 5: Run presenter tests and review only the new snapshot**

Run: `cargo test --test presenter project_search`

Then run: `cargo test --test presenter finder_overlay`

Expected: project-search tests pass and existing file-finder snapshots do not change. Accept only the new project-search snapshot after inspecting its path, line number, excerpt, scope label, and selected-row style.

- [ ] **Step 6: Checkpoint commit (only with explicit user authorization)**

```bash
git add src/presenter.rs src/controller/finder.rs src/controller/project_search.rs src/controller/mod.rs tests/presenter.rs tests/snapshots/presenter__presenter_project_search_results.snap
git commit -m "feat: render project search results\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

### Task 6: Keyboard journey, required docs, and verification

**Files:**
- Modify: `tests/e2e_keyboard.rs`
- Modify: `docs/keys.md`
- Modify: `docs/configuration.md`
- Modify: `docs/usage.md`
- Modify: `ARCHITECTURE.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add one positive e2e journey**

Create a fixture with the search term on a known later line. Launch the real binary, press `s`, type the term, wait for `path:line`, press Enter, and assert the content pane shows the opened file at that source location. The journey must observe each state before sending the next key; it must not press toggles to undo state it never asserted.

- [ ] **Step 2: Run the focused e2e test**

Run: `cargo test --test e2e_keyboard project_content_search_opens_selected_line -- --nocapture`

Expected: pass.

- [ ] **Step 3: Document the key and remapping surface**

Add to `docs/keys.md`:

```markdown
| `s` | **Search project contents**: search matching lines across the current root. Results show `file:line` plus an excerpt; `↑` / `↓` move, `Enter` opens the selected file at that source line, and `Esc` cancels. The scope follows `i`: ignored files are excluded while hidden, and included while revealed; `.git/` is never searched |
```

Add to `docs/configuration.md`’s Search & jump table:

```markdown
| | `project_search` | `s` | Search file contents across the current project scope |
```

Update the modal-controls paragraph to name the project-search prompt as fixed local controls.

- [ ] **Step 4: Add concise usage, architecture, and changelog entries**

In `docs/usage.md`, extend “Finding a file fast” (and its TOC label if renamed) with a “Searching file contents” subsection that states:

- `f` searches paths; `s` searches file contents;
- results are matching lines formatted as `path:line` plus excerpt;
- Enter reveals the file, switches to source content when needed, and lands on the line;
- `i` off excludes ignored files, `i` on includes them, and `.git/` is always excluded;
- non-text, unreadable, >1 MiB files are skipped; at most 500 rows are shown;
- search is literal smartcase, not regex.

Add `repo_search`, `project_search`, and `controller/project_search` rows/responsibilities to `ARCHITECTURE.md`, plus the background search worker to the async data-flow paragraph.

Under `CHANGELOG.md`’s `[Unreleased]`, add:

```markdown
### Added
- Project-content search: press `s` to find matching lines across the current root, then `Enter` to open a result at its source line. The scope follows `i`, so revealed ignored files become searchable while `.git/` stays excluded. → [usage](docs/usage.md#searching-file-contents) · [keys](docs/keys.md)
```

- [ ] **Step 5: Run the docs drift guards and focused feature tests**

Run:

```bash
cargo test keys_doc_table_documents_every_registry_action_ac21
cargo test configuration_doc_lists_every_remappable_intent
cargo test --test repo_search
cargo test --test project_search
cargo test --test e2e_keyboard project_content_search_opens_selected_line
```

Expected: all pass.

- [ ] **Step 6: Run the complete deterministic tier**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo audit
```

Expected: all commands exit 0. If `cargo audit` cannot reach its advisory source, report that separately; do not claim it passed.

- [ ] **Step 7: Manually exercise the requested flow**

Run `cargo run` in a fixture/repository and verify:

1. `s` opens the content-search popup without blocking navigation;
2. query edits replace stale results rather than flashing older-query output;
3. each row identifies a file and 1-based line;
4. Enter lands on that line in source content;
5. with `i` off, an ignored fixture does not appear;
6. after closing, pressing `i`, and reopening `s`, the ignored fixture appears;
7. `.git/` never appears;
8. Esc cancels without changing the current tree selection.

- [ ] **Step 8: Inspect the final diff and branch base**

Run:

```bash
git diff --check
git status --short
git log --oneline main..HEAD
git diff --stat main...HEAD
```

Expected: no whitespace errors; only feature-related source/tests/docs are changed; no stray commits are present.

- [ ] **Step 9: Final commit (only with explicit user authorization)**

```bash
git add src tests docs ARCHITECTURE.md CHANGELOG.md
git commit -m "feat: add project content search\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Plan self-review

- **Requirement coverage:** `s` binding is Task 4; current-root content scan and bounded guards are Task 2; `i` linkage is Tasks 1/3; file + line + excerpt presentation is Task 5; Enter source-line navigation is Task 3; asynchronous/stale-safe behavior is Task 3; minimum required docs are Task 6.
- **Scope:** One cohesive feature; no config keys, regex/glob syntax, persistent index, replace operation, or filesystem mutation are introduced.
- **Type consistency:** Scanner returns `SearchOutput { hits, limited }`; modal owns `SearchHit`; controller worker transports the same output; presenter receives preformatted rows through `FinderView` plus `FinderKind`.
- **Ambiguities resolved:** “Global” means the viewer’s current root, not the machine filesystem. `i` alone governs ignored-file inclusion; the separate `.` toggle does not narrow content search. `.git/` is excluded in every scope. A result row represents one matching line, not every occurrence on that line.
