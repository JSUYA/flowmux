// SPDX-License-Identifier: GPL-3.0-or-later
//! Backend-independent pane plumbing shared by the terminal
//! ([`super::terminal_pane_native`]) and browser ([`super::browser_pane`])
//! panes: the [`PaneCallbacks`] contract, the OSC-notification argv
//! wrapper, and a few constants.

use flowmux_core::{PaneId, SurfaceId};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

/// Alt+Enter sends `ESC CR` — used by the "insert newline" action so
/// agents that read it (e.g. Claude Code's multi-line prompt) get a
/// literal newline without submitting.
pub(crate) const ALT_ENTER_BYTES: &[u8] = b"\x1b\r";

#[derive(Clone)]
pub struct PaneCallbacks {
    pub on_notification: Rc<RefCell<dyn FnMut(PaneId, String, String)>>,
    pub on_bell: Rc<RefCell<dyn FnMut(PaneId)>>,
    pub on_child_exited: Rc<RefCell<dyn FnMut(PaneId, i32)>>,
    pub on_focus: Rc<RefCell<dyn FnMut(PaneId)>>,
    /// Per-pane close button on the Overlay + 'Close Pane' menu item.
    pub on_close_pane: Rc<RefCell<dyn FnMut(PaneId)>>,
    /// Right-click menu 'Split Right'.
    pub on_split_right: Rc<RefCell<dyn FnMut(PaneId)>>,
    /// Right-click menu 'Split Down'.
    pub on_split_down: Rc<RefCell<dyn FnMut(PaneId)>>,
    /// Pane-local surface tab activation.
    pub on_activate_surface: Rc<RefCell<dyn FnMut(PaneId, SurfaceId)>>,
    /// Pane-local new terminal tab.
    pub on_new_surface: Rc<RefCell<dyn FnMut(PaneId)>>,
    /// Pane-local new browser tab.
    pub on_new_browser_surface: Rc<RefCell<dyn FnMut(PaneId)>>,
    /// Pane-local close tab.
    pub on_close_surface: Rc<RefCell<dyn FnMut(PaneId, SurfaceId)>>,
    /// Pane-local rename tab.
    pub on_rename_surface: Rc<RefCell<dyn FnMut(PaneId, SurfaceId)>>,
    /// Tab right-click "Show in folder" → open file manager at the
    /// terminal surface's current working directory. Only invoked from
    /// terminal tab popovers; browser tabs skip the menu entirely.
    pub on_show_surface_folder: Rc<RefCell<dyn FnMut(PaneId, SurfaceId)>>,
    /// Per-surface "Copy path" / "Copy URL" handler. The dispatcher
    /// reads the surface kind and copies cwd or URL accordingly, so
    /// the same callback is reused by both terminal and browser
    /// right-click menus.
    pub on_copy_surface_text: Rc<RefCell<dyn FnMut(PaneId, SurfaceId)>>,
    /// Reorder a tab within the same pane by drag and drop. The third argument
    /// is the final 0-based index after the move, clamped if it exceeds length.
    pub on_reorder_surface: Rc<RefCell<dyn FnMut(PaneId, SurfaceId, usize)>>,
    /// A tab drag ended without landing on another tab drop target. The caller
    /// moves that live surface into a new top-level window and removes it from
    /// the source pane.
    pub on_tab_drag_to_new_window: Rc<RefCell<dyn FnMut(PaneId, SurfaceId)>>,
    /// Shared across all surface tabs in one window for the duration of a drag.
    /// The source tab uses this to distinguish a true no-target drag from a
    /// rejected drop on a known tab (self/cross-pane/invalid payload).
    pub tab_drag_drop_seen: Rc<Cell<bool>>,
    /// A terminal surface changed its cwd.
    pub on_terminal_cwd_changed: Rc<RefCell<dyn FnMut(PaneId, SurfaceId, PathBuf)>>,
    /// WebKit reported that a browser pane navigated to a new URL.
    pub on_browser_uri_changed: Rc<RefCell<dyn FnMut(PaneId, SurfaceId, String)>>,
    /// WebKit reported that a browser pane's page title changed.
    pub on_browser_title_changed: Rc<RefCell<dyn FnMut(PaneId, SurfaceId, String)>>,
    /// A terminal surface emitted an OSC 0/2 window title, often from programs
    /// such as vi, claude, codex, or tmux. Empty titles are ignored by the
    /// caller.
    pub on_terminal_title_changed: Rc<RefCell<dyn FnMut(PaneId, SurfaceId, String)>>,
    /// Return the current user options. Used when creating a new BrowserPane to
    /// choose the engine and apply zoom immediately after widget creation. This
    /// cheaply clones the `Rc<RefCell<Options>>` held by WindowController, so
    /// dialog updates are visible on the next call.
    pub read_options: Rc<dyn Fn() -> flowmux_config::options::Options>,
    /// Return the surface's current 0-based index within the same pane. Tab DnD
    /// uses PaneRegistry::surface_tabs to compute final_index from the source
    /// and target relative positions.
    pub position_of_surface_in_pane: Rc<dyn Fn(PaneId, SurfaceId) -> Option<usize>>,
    /// Called when Ctrl+click selects a URL inside the terminal. The caller
    /// opens that URL in a new browser tab in the same pane
    /// (GtkCommand::OpenUrlInBrowserTab). The URL arrives with trailing
    /// punctuation already trimmed.
    pub on_open_url: Rc<RefCell<dyn FnMut(PaneId, String)>>,
}

/// Prepend `flowmuxctl pty-tee --pane <id> --surface <id> --` in front of
/// the user's shell argv so OSC 9 / 99 / 777 escapes emitted by
/// terminal-side agents (Claude Code, Codex, OpenCode, …) reach the
/// daemon's `Request::Notify` path. This is the distribution-agnostic
/// interception point for desktop notifications and is independent of the
/// terminal rendering backend.
///
/// Falls back to the original argv when `flowmuxctl` cannot be located.
pub(crate) fn wrap_argv_with_pty_tee(
    argv: Vec<String>,
    pane: PaneId,
    surface: SurfaceId,
) -> Vec<String> {
    let Some(ctl) = flowmux_terminal::find_flowmuxctl() else {
        tracing::warn!(
            "flowmuxctl not found next to the GUI binary; OSC 9/99/777 alarms \
             from terminal-side agents will be silently dropped until it is \
             installed. Falling back to a direct shell spawn."
        );
        return argv;
    };
    let mut wrapped = Vec::with_capacity(argv.len() + 6);
    wrapped.push(ctl.display().to_string());
    wrapped.push("pty-tee".to_string());
    wrapped.push("--pane".to_string());
    wrapped.push(pane.to_string());
    wrapped.push("--surface".to_string());
    wrapped.push(surface.to_string());
    wrapped.push("--".to_string());
    wrapped.extend(argv);
    wrapped
}
