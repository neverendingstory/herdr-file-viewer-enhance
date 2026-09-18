//! Project-content search (`s`) — modal key handling, worker dispatch, result application, and
//! confirmation through the shared open-target path.

use super::*;
use crate::open_target::OpenTarget;
use crate::repo_search::SearchHit;

impl Controller {
    /// Open a fresh content-search modal using the ignored-file scope visible at this instant.
    pub(super) fn open_project_search(&mut self) -> Effects {
        self.project_search_seq += 1; // invalidate a completion from an earlier modal instance
        self.modal = Modal::ProjectSearch(ProjectSearchState::new(self.show_ignored));
        self.last_click = None;
        Effects::redraw()
    }

    /// Whether project-content search currently owns keyboard input.
    pub fn project_search_open(&self) -> bool {
        self.modal.project_search().is_some()
    }

    /// Current result rows, exposed for deterministic controller integration tests.
    pub fn project_search_hits(&self) -> Option<&[SearchHit]> {
        self.modal.project_search().map(ProjectSearchState::hits)
    }

    /// Route one raw key while project-content search is open.
    pub fn handle_project_search_key(&mut self, key: KeyEvent) -> Effects {
        if self.modal.project_search().is_none() {
            return Effects::noop();
        }

        match key.code {
            KeyCode::Char(c) if key.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
                self.modal
                    .project_search_mut()
                    .expect("checked above")
                    .push(c);
                self.dispatch_project_search();
                Effects::redraw()
            }
            KeyCode::Backspace => {
                self.modal
                    .project_search_mut()
                    .expect("checked above")
                    .backspace();
                self.dispatch_project_search();
                Effects::redraw()
            }
            KeyCode::Up => {
                self.modal
                    .project_search_mut()
                    .expect("checked above")
                    .move_selection(-1);
                Effects::redraw()
            }
            KeyCode::Down => {
                self.modal
                    .project_search_mut()
                    .expect("checked above")
                    .move_selection(1);
                Effects::redraw()
            }
            KeyCode::Left => {
                self.modal
                    .project_search_mut()
                    .expect("checked above")
                    .scroll_left();
                Effects::redraw()
            }
            KeyCode::Right => {
                self.modal
                    .project_search_mut()
                    .expect("checked above")
                    .scroll_right();
                Effects::redraw()
            }
            KeyCode::Enter => self.confirm_project_search(),
            KeyCode::Esc => {
                self.project_search_seq += 1;
                self.modal = Modal::None;
                self.last_click = None;
                Effects::redraw()
            }
            _ => Effects::noop(),
        }
    }

    fn dispatch_project_search(&mut self) {
        self.project_search_seq += 1;
        let seq = self.project_search_seq;
        let Some(state) = self.modal.project_search() else {
            return;
        };
        if state.query().is_empty() {
            return;
        }
        let job = ProjectSearchJob {
            seq,
            root: self.root.clone(),
            query: state.query().to_string(),
            include_ignored: state.include_ignored(),
        };
        if self.project_search_tx.send(job).is_err()
            && let Some(state) = self.modal.project_search_mut()
        {
            state.apply(crate::repo_search::SearchOutput::default());
        }
    }

    fn confirm_project_search(&mut self) -> Effects {
        let Some(hit) = self
            .modal
            .project_search()
            .and_then(ProjectSearchState::selected)
            .cloned()
        else {
            return Effects::noop();
        };

        self.project_search_seq += 1;
        self.modal = Modal::None;
        self.last_click = None;
        self.apply_open_target(&OpenTarget {
            path: hit.path,
            line: Some(hit.line),
            end_line: None,
        });
        Effects::redraw()
    }

    /// Borrow-free projection onto the shared finder popup surface.
    pub(super) fn project_search_view(&self) -> Option<FinderView> {
        let state = self.modal.project_search()?;
        Some(FinderView {
            kind: FinderKind::ProjectContent {
                searching: state.searching(),
                limited: state.limited(),
                include_ignored: state.include_ignored(),
            },
            query: state.query().to_string(),
            matches: state
                .hits()
                .iter()
                .map(|hit| format!("{}:{}  {}", hit.path, hit.line, hit.excerpt))
                .collect(),
            cursor: state.cursor(),
            hscroll: state.hscroll(),
        })
    }
}
