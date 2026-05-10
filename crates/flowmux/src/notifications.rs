// SPDX-License-Identifier: GPL-3.0-or-later
//! In-process notification log shown in the sidebar's bell popover.
//!
//! flowmux already forwards every `Request::Notify` to the desktop via
//! `org.freedesktop.Notifications` (flowmux-notify). The GUI also keeps
//! a small in-memory transcript so the user can scroll past
//! notifications even after the OS toast fades — pressing the bell
//! button at the top of the sidebar opens a popover listing them.
//!
//! Each entry remembers the source `PaneId` / `WorkspaceId`, so
//! clicking a popover row routes back to that pane (cmux parity:
//! `openNotification → focusTabFromNotification`). Clicking also flips
//! `read = true`, which the popover renders with reduced opacity.

use flowmux_core::{NotificationId, NotificationLevel, PaneId, SurfaceId, WorkspaceId};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Cap on retained entries. A chatty agent (Claude Code's per-step OSC
/// 9 stream) can otherwise grow this unbounded over a long session.
/// 200 matches cmux's `TerminalNotificationStore` policy.
const MAX_RETAINED: usize = 200;

#[derive(Debug, Clone)]
pub struct NotificationEntry {
    pub id: NotificationId,
    pub title: String,
    pub body: String,
    pub level: NotificationLevel,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// True once the user has seen this entry — either by clicking it in
    /// the popover or by it being delivered while its workspace+pane was
    /// already focused. Renders dimmed.
    pub read: bool,
    /// Source pane for the click-to-focus route. `None` when the
    /// notifier did not specify a pane (e.g. global toasts).
    pub pane: Option<PaneId>,
    /// Specific tab inside `pane` that triggered the event so the
    /// click router can switch tabs when the user is currently
    /// looking at a different surface in the same pane.
    pub surface: Option<SurfaceId>,
    /// Workspace of `pane`, resolved by the IPC handler before the entry
    /// reached the GUI thread. Without it we cannot route clicks back to
    /// the right side-panel row.
    pub workspace: Option<WorkspaceId>,
    /// FDO `org.freedesktop.Notifications.Notify` id returned by the
    /// notification daemon when the desktop toast was sent. Stored so
    /// that — once the user reads this entry inside the bell popover —
    /// flowmux can issue `CloseNotification(desktop_id)` to the FDO
    /// daemon and shrink the GNOME / KDE dock badge accordingly.
    /// `None` when the toast was suppressed (e.g. source pane already
    /// focused) or when the FDO daemon was unreachable.
    pub desktop_id: Option<u32>,
}

/// Thread-local notification store backing the sidebar bell popover and
/// the click-to-focus router.
///
/// The store is `!Send` on purpose — only the GTK main thread writes to
/// it. Background paths (IPC handler, OSC reader) reach it via
/// `GtkCommand::AddNotification` so the bridge is the single
/// serialization point.
#[derive(Clone, Default)]
pub struct NotificationStore {
    inner: Rc<RefCell<VecDeque<NotificationEntry>>>,
}

impl NotificationStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a fresh entry. When the buffer is full the oldest entry
    /// is dropped to keep memory bounded under chatty agent streams.
    /// Returns the assigned id so the caller can later reference it.
    pub fn push(
        &self,
        title: String,
        body: String,
        level: NotificationLevel,
        pane: Option<PaneId>,
        surface: Option<SurfaceId>,
        workspace: Option<WorkspaceId>,
    ) -> NotificationId {
        let id = NotificationId::new();
        let mut entries = self.inner.borrow_mut();
        if entries.len() >= MAX_RETAINED {
            entries.pop_front();
        }
        entries.push_back(NotificationEntry {
            id,
            title,
            body,
            level,
            created_at: chrono::Utc::now(),
            read: false,
            pane,
            surface,
            workspace,
            desktop_id: None,
        });
        id
    }

    /// Flip `read = true` for `id`. Returns `true` when the entry was
    /// found and changed (so callers can avoid redundant re-renders).
    pub fn mark_read(&self, id: NotificationId) -> bool {
        let mut entries = self.inner.borrow_mut();
        if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
            if !e.read {
                e.read = true;
                return true;
            }
        }
        false
    }

    /// Record the FDO desktop notification id assigned to `id`. The
    /// IPC handler calls this after the daemon returns
    /// `Response::Notified` so the popover can later ask the daemon to
    /// dismiss the toast when the user reads it inside flowmux.
    pub fn set_desktop_id(&self, id: NotificationId, desktop_id: u32) {
        let mut entries = self.inner.borrow_mut();
        if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
            e.desktop_id = Some(desktop_id);
        }
    }

    /// Flip every unread entry to `read = true` and return the
    /// `desktop_id` of each one that previously carried a desktop
    /// notification. The caller is expected to forward those ids to
    /// `Request::CloseDesktopNotification` so the FDO notification
    /// center clears them out. Returns an empty vec when nothing
    /// changed — handy for skipping a no-op IPC roundtrip.
    pub fn mark_all_unread_read(&self) -> Vec<u32> {
        let mut entries = self.inner.borrow_mut();
        let mut closed = Vec::new();
        for e in entries.iter_mut() {
            if !e.read {
                e.read = true;
                if let Some(did) = e.desktop_id {
                    closed.push(did);
                }
            }
        }
        closed
    }

    pub fn find(&self, id: NotificationId) -> Option<NotificationEntry> {
        self.inner.borrow().iter().find(|e| e.id == id).cloned()
    }

    /// Return a fresh `Vec` snapshot in insertion order. The caller owns
    /// the list and may render newest-first.
    pub fn entries(&self) -> Vec<NotificationEntry> {
        self.inner.borrow().iter().cloned().collect()
    }

    /// Number of unread entries. Currently used by tests; kept public so
    /// the sidebar can show a future count badge without a second pass.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn unread_count(&self) -> usize {
        self.inner.borrow().iter().filter(|e| !e.read).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> NotificationStore {
        NotificationStore::new()
    }

    #[test]
    fn push_returns_id_that_resolves_back_to_entry() {
        let s = store();
        let id = s.push(
            "Claude".into(),
            "done".into(),
            NotificationLevel::Info,
            None,
            None,
            None,
        );
        let found = s.find(id).expect("entry should be findable by id");
        assert_eq!(found.title, "Claude");
        assert_eq!(found.body, "done");
        assert!(!found.read);
    }

    #[test]
    fn mark_read_is_idempotent_and_flips_only_once() {
        let s = store();
        let id = s.push(
            "x".into(),
            "y".into(),
            NotificationLevel::Info,
            None,
            None,
            None,
        );
        assert!(s.mark_read(id), "first mark_read should report a change");
        assert!(!s.mark_read(id), "second mark_read on same id is a no-op");
        assert!(s.find(id).unwrap().read);
    }

    #[test]
    fn mark_read_with_unknown_id_is_safe_noop() {
        let s = store();
        assert!(!s.mark_read(NotificationId::new()));
        assert_eq!(s.unread_count(), 0);
    }

    #[test]
    fn entries_preserve_insertion_order_and_unread_count_tracks_reads() {
        let s = store();
        let a = s.push(
            "a".into(),
            "".into(),
            NotificationLevel::Info,
            None,
            None,
            None,
        );
        let b = s.push(
            "b".into(),
            "".into(),
            NotificationLevel::Info,
            None,
            None,
            None,
        );
        let c = s.push(
            "c".into(),
            "".into(),
            NotificationLevel::Info,
            None,
            None,
            None,
        );
        let entries = s.entries();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id, a);
        assert_eq!(entries[1].id, b);
        assert_eq!(entries[2].id, c);
        assert_eq!(s.unread_count(), 3);
        s.mark_read(b);
        assert_eq!(s.unread_count(), 2);
        s.mark_read(a);
        s.mark_read(c);
        assert_eq!(s.unread_count(), 0);
    }

    #[test]
    fn push_records_pane_and_workspace_for_click_routing() {
        let s = store();
        let pane = PaneId::new();
        let surface = SurfaceId::new();
        let ws = WorkspaceId::new();
        let id = s.push(
            "claude".into(),
            "ready".into(),
            NotificationLevel::AttentionNeeded,
            Some(pane),
            Some(surface),
            Some(ws),
        );
        let e = s.find(id).unwrap();
        assert_eq!(e.pane, Some(pane));
        assert_eq!(e.surface, Some(surface));
        assert_eq!(e.workspace, Some(ws));
        assert_eq!(e.level, NotificationLevel::AttentionNeeded);
    }

    #[test]
    fn push_drops_oldest_entry_once_cap_is_reached() {
        let s = store();
        // Insert MAX_RETAINED + 5 — the first 5 should age out and the
        // newest 5 must keep their order at the tail.
        let mut ids = Vec::new();
        for i in 0..(MAX_RETAINED + 5) {
            ids.push(s.push(
                format!("entry {i}"),
                "".into(),
                NotificationLevel::Info,
                None,
                None,
                None,
            ));
        }
        let entries = s.entries();
        assert_eq!(entries.len(), MAX_RETAINED);
        // The first 5 ids should no longer be findable.
        for stale in &ids[..5] {
            assert!(s.find(*stale).is_none());
        }
        // The newest id should still be at the tail.
        assert_eq!(entries.last().unwrap().id, *ids.last().unwrap());
    }

    #[test]
    fn duplicate_push_for_same_pane_creates_distinct_entries() {
        let s = store();
        let pane = PaneId::new();
        let ws = WorkspaceId::new();
        let a = s.push(
            "claude".into(),
            "step 1".into(),
            NotificationLevel::Info,
            Some(pane),
            None,
            Some(ws),
        );
        let b = s.push(
            "claude".into(),
            "step 2".into(),
            NotificationLevel::Info,
            Some(pane),
            None,
            Some(ws),
        );
        assert_ne!(a, b);
        assert_eq!(s.entries().len(), 2);
    }

    #[test]
    fn set_desktop_id_attaches_fdo_id_to_existing_entry() {
        let s = store();
        let id = s.push(
            "Claude".into(),
            "ready".into(),
            NotificationLevel::AttentionNeeded,
            None,
            None,
            None,
        );
        s.set_desktop_id(id, 42);
        assert_eq!(s.find(id).unwrap().desktop_id, Some(42));
    }

    #[test]
    fn set_desktop_id_for_unknown_id_is_safe_noop() {
        let s = store();
        // No panic, no insert — just silently ignored. Mirrors the
        // mark_read contract for entries that aged out under the cap.
        s.set_desktop_id(NotificationId::new(), 7);
        assert_eq!(s.entries().len(), 0);
    }

    #[test]
    fn mark_all_unread_read_flips_unread_and_returns_their_desktop_ids() {
        let s = store();
        let a = s.push(
            "a".into(),
            "".into(),
            NotificationLevel::AttentionNeeded,
            None,
            None,
            None,
        );
        let b = s.push(
            "b".into(),
            "".into(),
            NotificationLevel::AttentionNeeded,
            None,
            None,
            None,
        );
        let c = s.push(
            "c".into(),
            "".into(),
            NotificationLevel::AttentionNeeded,
            None,
            None,
            None,
        );
        // Entry `a` carries no desktop_id (e.g. toast was suppressed),
        // so it should be marked read but NOT show up in the close
        // list. `b` and `c` should both round-trip their ids.
        s.set_desktop_id(b, 11);
        s.set_desktop_id(c, 22);
        // Pre-mark `b` as already read so the loop must skip it and
        // not double-emit its desktop_id.
        s.mark_read(b);
        let to_close = s.mark_all_unread_read();
        assert_eq!(s.unread_count(), 0);
        // Order follows insertion order; only the previously-unread
        // ones with a desktop_id should appear.
        assert_eq!(to_close, vec![22]);
        // `a` still has no desktop_id but is now read.
        assert!(s.find(a).unwrap().read);
        assert!(s.find(c).unwrap().read);
    }

    #[test]
    fn mark_all_unread_read_returns_empty_when_nothing_changed() {
        let s = store();
        // No entries → empty list, no panic.
        assert!(s.mark_all_unread_read().is_empty());
        // Insert and mark read; second sweep is a no-op.
        let id = s.push(
            "x".into(),
            "".into(),
            NotificationLevel::Info,
            None,
            None,
            None,
        );
        s.set_desktop_id(id, 99);
        assert_eq!(s.mark_all_unread_read(), vec![99]);
        assert!(s.mark_all_unread_read().is_empty());
    }

    #[test]
    fn push_records_distinct_surfaces_within_same_pane() {
        let s = store();
        let pane = PaneId::new();
        let tab_a = SurfaceId::new();
        let tab_b = SurfaceId::new();
        let id_a = s.push(
            "Claude".into(),
            "tab A done".into(),
            NotificationLevel::Info,
            Some(pane),
            Some(tab_a),
            None,
        );
        let id_b = s.push(
            "Codex".into(),
            "tab B done".into(),
            NotificationLevel::Info,
            Some(pane),
            Some(tab_b),
            None,
        );
        assert_eq!(s.find(id_a).unwrap().surface, Some(tab_a));
        assert_eq!(s.find(id_b).unwrap().surface, Some(tab_b));
    }
}
