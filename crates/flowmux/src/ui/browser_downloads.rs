// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
enum DownloadPhase {
    InProgress,
    Cancelling,
    Complete,
    Cancelled,
    Failed(String),
}

impl DownloadPhase {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Complete | Self::Cancelled | Self::Failed(_))
    }
}

#[derive(Clone, Debug)]
struct DownloadLifecycle {
    phase: DownloadPhase,
}

impl Default for DownloadLifecycle {
    fn default() -> Self {
        Self {
            phase: DownloadPhase::InProgress,
        }
    }
}

impl DownloadLifecycle {
    fn phase(&self) -> &DownloadPhase {
        &self.phase
    }

    fn request_cancel(&mut self) -> bool {
        if self.phase != DownloadPhase::InProgress {
            return false;
        }
        self.phase = DownloadPhase::Cancelling;
        true
    }

    fn finish(&mut self) -> bool {
        if self.phase.is_terminal() {
            return false;
        }
        self.phase = if self.phase == DownloadPhase::Cancelling {
            DownloadPhase::Cancelled
        } else {
            DownloadPhase::Complete
        };
        true
    }

    fn fail(&mut self, error: String) -> bool {
        if self.phase.is_terminal() {
            return false;
        }
        self.phase = if self.phase == DownloadPhase::Cancelling {
            DownloadPhase::Cancelled
        } else {
            DownloadPhase::Failed(error)
        };
        true
    }
}

#[derive(Default)]
struct DownloadCollection {
    next_id: u64,
    active_count: usize,
    entries: HashMap<u64, DownloadLifecycle>,
}

impl DownloadCollection {
    fn insert(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.active_count += 1;
        self.entries.insert(id, DownloadLifecycle::default());
        id
    }

    fn request_cancel(&mut self, id: u64) -> bool {
        self.entries
            .get_mut(&id)
            .is_some_and(DownloadLifecycle::request_cancel)
    }

    fn finish(&mut self, id: u64) -> bool {
        let transitioned = self
            .entries
            .get_mut(&id)
            .is_some_and(DownloadLifecycle::finish);
        if transitioned {
            self.active_count -= 1;
        }
        transitioned
    }

    fn fail(&mut self, id: u64, error: String) -> bool {
        let transitioned = self
            .entries
            .get_mut(&id)
            .is_some_and(|entry| entry.fail(error));
        if transitioned {
            self.active_count -= 1;
        }
        transitioned
    }

    fn remove_terminal(&mut self, id: u64) -> bool {
        if !self
            .entries
            .get(&id)
            .is_some_and(|entry| entry.phase().is_terminal())
        {
            return false;
        }
        self.entries.remove(&id);
        true
    }

    fn clear_terminal(&mut self) -> Vec<u64> {
        let mut terminal: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(id, entry)| entry.phase().is_terminal().then_some(*id))
            .collect();
        terminal.sort_unstable();
        for id in &terminal {
            self.entries.remove(id);
        }
        terminal
    }

    fn active_count(&self) -> usize {
        self.active_count
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_finish_never_becomes_complete() {
        let mut lifecycle = DownloadLifecycle::default();
        assert!(lifecycle.request_cancel());
        assert!(lifecycle.finish());
        assert_eq!(lifecycle.phase(), &DownloadPhase::Cancelled);
    }

    #[test]
    fn cancelled_failure_is_reported_as_cancelled() {
        let mut lifecycle = DownloadLifecycle::default();
        lifecycle.request_cancel();
        assert!(lifecycle.fail("network stopped".into()));
        assert_eq!(lifecycle.phase(), &DownloadPhase::Cancelled);
    }

    #[test]
    fn failure_cannot_be_overwritten_by_finished() {
        let mut lifecycle = DownloadLifecycle::default();
        assert!(lifecycle.fail("connection reset".into()));
        assert!(!lifecycle.finish());
        assert_eq!(
            lifecycle.phase(),
            &DownloadPhase::Failed("connection reset".into())
        );
    }

    #[test]
    fn normal_finish_is_complete() {
        let mut lifecycle = DownloadLifecycle::default();
        assert!(lifecycle.finish());
        assert_eq!(lifecycle.phase(), &DownloadPhase::Complete);
    }

    #[test]
    fn overlapping_downloads_decrement_active_count_once() {
        let mut collection = DownloadCollection::default();
        let first = collection.insert();
        let second = collection.insert();
        assert_eq!(collection.active_count(), 2);
        assert!(collection.finish(first));
        assert_eq!(collection.active_count(), 1);
        assert!(!collection.finish(first));
        assert_eq!(collection.active_count(), 1);
        assert!(collection.fail(second, "offline".into()));
        assert_eq!(collection.active_count(), 0);
    }

    #[test]
    fn clear_terminal_keeps_active_entries() {
        let mut collection = DownloadCollection::default();
        let active = collection.insert();
        let finished = collection.insert();
        collection.finish(finished);
        assert_eq!(collection.clear_terminal(), vec![finished]);
        assert_eq!(collection.len(), 1);
        assert!(!collection.remove_terminal(active));
        assert_eq!(collection.active_count(), 1);
    }
}
