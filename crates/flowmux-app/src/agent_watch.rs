// SPDX-License-Identifier: GPL-3.0-or-later
//! Detect when AI-coding-agent commands finish in any terminal pane.
//!
//! Mirrors cmux's signature attention behavior: when claude / codex /
//! opencode finishes a turn the user wants to be told. flowmux polls
//! the descendant tree of each terminal pane's shell every couple of
//! seconds and emits a one-shot `AgentCompleted` event whenever an
//! agent process that *was* there a tick ago is no longer there.
//!
//! Comparison is by `comm` (the basename of the executable from
//! `/proc/<pid>/comm`) so it doesn't matter which directory the
//! agent was invoked from. We deliberately avoid diffing against
//! exit-status, since agents typically self-restart between turns
//! while keeping the same parent shell — what we want to capture is
//! "agent isn't running right now", not "agent crashed".

use crate::bridge::{Bridge, GtkCommand};
use crate::ui::workspace_view::PaneRegistry;
use flowmux_core::PaneId;
use gtk::glib;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

/// Process-name prefixes we treat as agent commands.
const AGENT_PREFIXES: &[&str] = &["claude", "codex", "opencode"];

#[derive(Default)]
struct AgentWatcher {
    /// Per-pane set of agent comm strings observed last tick.
    state: HashMap<PaneId, HashSet<String>>,
}

impl AgentWatcher {
    fn poll(&mut self, registry: &PaneRegistry) -> Vec<(PaneId, String)> {
        let mut events = Vec::new();
        let mut now: HashMap<PaneId, HashSet<String>> = HashMap::new();
        for (pane_id, term) in registry.terminals.iter() {
            let agents = collect_agents(term);
            if let Some(prev) = self.state.get(pane_id) {
                for gone in prev.difference(&agents) {
                    events.push((*pane_id, gone.clone()));
                }
            }
            now.insert(*pane_id, agents);
        }
        // Forget panes that no longer exist (closed tabs etc).
        self.state = now;
        events
    }
}

fn collect_agents(term: &crate::ui::terminal_pane::TerminalPane) -> HashSet<String> {
    let mut out = HashSet::new();
    let Some(shell_pid) = term.pid.get() else { return out };
    let Ok(descendants) = flowmux_procmon::descendants(shell_pid as u32) else { return out };
    for pid in descendants {
        if pid as i32 == shell_pid {
            continue; // skip the shell itself
        }
        if let Some(comm) = flowmux_procmon::comm_of(pid) {
            if AGENT_PREFIXES.iter().any(|prefix| comm.starts_with(prefix)) {
                out.insert(comm);
            }
        }
    }
    out
}

/// Install a glib timeout that polls the registry every two seconds
/// and forwards any `AgentCompleted` events through the bridge.
pub fn install(pane_registry: Rc<RefCell<PaneRegistry>>, bridge: Bridge) {
    let watcher: Rc<RefCell<AgentWatcher>> = Rc::new(RefCell::new(AgentWatcher::default()));
    glib::timeout_add_local(Duration::from_secs(2), move || {
        let registry = pane_registry.borrow();
        let events = watcher.borrow_mut().poll(&registry);
        drop(registry);
        for (pane, name) in events {
            let bridge = bridge.clone();
            glib::MainContext::default().spawn_local(async move {
                let _ = bridge
                    .tx
                    .send(GtkCommand::AgentCompleted { pane, name })
                    .await;
            });
        }
        glib::ControlFlow::Continue
    });
}
