//! VTE-backed terminal pane.
//!
//! Spawns the user's shell in a PTY and surfaces:
//!
//! * `notification-received` (OSC 99 / Konsole) → forwarded as a
//!   structured notification to the app handler;
//! * `bell` (BEL) → optional attention signal;
//! * `child-exited` → caller decides whether to recycle the pane.
//!
//! For OSC 9 / 777 cmux supports, those are not fired by VTE as
//! distinct signals — agents wishing to use them should pipe through
//! `flowmux notify-stream` (which uses the same parser the GUI uses).
//! We will revisit when libghostty backend lands.

use flowmux_core::PaneId;
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use vte::prelude::*;

#[derive(Clone)]
pub struct TerminalPane {
    pub id: PaneId,
    pub widget: vte::Terminal,
}

#[derive(Clone)]
pub struct PaneCallbacks {
    pub on_notification: Rc<RefCell<dyn FnMut(PaneId, String, String)>>,
    pub on_bell: Rc<RefCell<dyn FnMut(PaneId)>>,
    pub on_child_exited: Rc<RefCell<dyn FnMut(PaneId, i32)>>,
}

impl TerminalPane {
    /// Build a fresh terminal widget and spawn `argv` in `cwd`. If
    /// `argv` is empty we fall back to the user's `$SHELL`.
    pub fn spawn(
        id: PaneId,
        argv: Vec<String>,
        cwd: Option<std::path::PathBuf>,
        callbacks: PaneCallbacks,
    ) -> Self {
        let term = vte::Terminal::new();
        term.set_hexpand(true);
        term.set_vexpand(true);
        term.set_scrollback_lines(10_000);
        term.set_audible_bell(false);

        // OSC 99 (Konsole-format) is not exposed as a signal on Ubuntu's
        // VTE 0.76 build — the `notification-received` signal is a
        // Konsole extension compiled out in upstream VTE. We capture
        // OSC notifications via the `flowmux notify-stream` CLI today,
        // and a PTY-tee path is planned in flowmux-terminal so the GUI
        // can subscribe directly without wrapping every command.
        let _unused_notification_cb = &callbacks.on_notification;

        // BEL — generic attention.
        {
            let cb = callbacks.on_bell.clone();
            let id = id;
            term.connect_bell(move |_term| {
                (cb.borrow_mut())(id);
            });
        }

        // Process exit.
        {
            let cb = callbacks.on_child_exited.clone();
            let id = id;
            term.connect_child_exited(move |_term, status| {
                (cb.borrow_mut())(id, status);
            });
        }

        let argv: Vec<String> = if argv.is_empty() {
            vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())]
        } else {
            argv
        };
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let cwd_str = cwd.as_ref().and_then(|p| p.to_str());

        term.spawn_async(
            vte::PtyFlags::DEFAULT,
            cwd_str,
            &argv_refs,
            &[], // envv: inherit
            glib::SpawnFlags::DEFAULT,
            || {}, // child setup (runs in child after fork)
            -1,    // no timeout
            gtk::gio::Cancellable::NONE,
            |result| {
                if let Err(e) = result {
                    tracing::warn!(error = %e, "vte spawn failed");
                }
            },
        );

        Self { id, widget: term }
    }

    pub fn feed(&self, bytes: &[u8]) {
        self.widget.feed_child(bytes);
    }
}
