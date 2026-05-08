// SPDX-License-Identifier: GPL-3.0-or-later
//! Terminal backend abstraction.
//!
//! flowmux renders panes through a [`TerminalBackend`] so we can swap
//! implementations without touching the application or IPC layers:
//!
//! * `vte` (default) — the VTE 2.91 GTK4 widget used by GNOME Terminal,
//!   Tilix, and Black Box. Mature, OSC sequences mostly handled.
//! * `ghostty` (planned) — libghostty embedded into a GTK widget. Same
//!   renderer cmux uses on macOS, for output parity.
//!
//! See `docs/upstream-mapping/terminal.md` for the parity matrix.

use flowmux_core::{PaneId, WorkspaceId};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("pane not found: {0}")]
    NotFound(PaneId),
    #[cfg(feature = "vte")]
    #[error("glib: {0}")]
    Glib(String),
}

#[derive(Debug, Clone)]
pub struct SpawnSpec<'a> {
    pub argv: &'a [&'a str],
    pub cwd: Option<&'a Path>,
    pub env: &'a [(&'a str, &'a str)],
}

/// Env vars flowmux injects into every PTY spawn so terminal-side agents
/// (claude, codex, opencode, …) can discover their own pane and the
/// daemon socket without explicit flags. Mirrors cmux's
/// `GhosttyTerminalView` env injection — we only swap the `CMUX_` prefix
/// for `FLOWMUX_`.
///
/// Variables produced:
/// * `FLOWMUX_PANE_ID` / `FLOWMUX_SURFACE_ID` — same value (pane is the
///   surface alias on flowmux, consistent with cmux's panel↔surface alias).
/// * `FLOWMUX_WORKSPACE_ID` / `FLOWMUX_TAB_ID` — same value.
/// * `FLOWMUX_SOCKET_PATH` — the daemon's Unix socket path.
/// * `FLOWMUX_BUNDLED_CLI_PATH` — only when the caller knows where the
///   `flowmux` binary lives (e.g. derived from `current_exe()` in app).
pub fn agent_pty_env(
    pane: PaneId,
    workspace: WorkspaceId,
    socket: &Path,
    bundled_cli: Option<&Path>,
) -> Vec<(String, String)> {
    let pane_s = pane.to_string();
    let workspace_s = workspace.to_string();
    let mut out = Vec::with_capacity(6);
    out.push(("FLOWMUX_PANE_ID".to_string(), pane_s.clone()));
    out.push(("FLOWMUX_SURFACE_ID".to_string(), pane_s));
    out.push(("FLOWMUX_WORKSPACE_ID".to_string(), workspace_s.clone()));
    out.push(("FLOWMUX_TAB_ID".to_string(), workspace_s));
    out.push((
        "FLOWMUX_SOCKET_PATH".to_string(),
        socket.display().to_string(),
    ));
    if let Some(p) = bundled_cli {
        out.push((
            "FLOWMUX_BUNDLED_CLI_PATH".to_string(),
            p.display().to_string(),
        ));
    }
    out
}

/// Convenience: collapse `[(k, v)]` env pairs into the `KEY=VALUE`
/// strings expected by GLib / VTE `spawn_async` envv arrays.
pub fn env_to_kv_strings(env: &[(String, String)]) -> Vec<String> {
    env.iter().map(|(k, v)| format!("{k}={v}")).collect()
}

pub trait TerminalBackend {
    /// Spawn a process in a fresh pane and return its id.
    fn spawn(&mut self, spec: SpawnSpec<'_>) -> Result<PaneId, TerminalError>;
    /// Send keystrokes to a pane (raw bytes; caller handles escape).
    fn send(&mut self, pane: PaneId, bytes: &[u8]) -> Result<(), TerminalError>;
    /// Resize to (rows, cols).
    fn resize(&mut self, pane: PaneId, rows: u16, cols: u16) -> Result<(), TerminalError>;
    /// Close pane and reap child.
    fn close(&mut self, pane: PaneId) -> Result<(), TerminalError>;
}

#[cfg(feature = "vte")]
pub mod vte_backend;

#[cfg(feature = "ghostty")]
pub mod ghostty_backend;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn collect<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn agent_pty_env_includes_pane_workspace_socket() {
        let pane = PaneId::new();
        let ws = WorkspaceId::new();
        let socket = PathBuf::from("/run/user/1000/flowmux.sock");

        let env = agent_pty_env(pane, ws, &socket, None);

        assert_eq!(collect(&env, "FLOWMUX_PANE_ID"), Some(pane.to_string().as_str()));
        assert_eq!(
            collect(&env, "FLOWMUX_SURFACE_ID"),
            Some(pane.to_string().as_str())
        );
        assert_eq!(
            collect(&env, "FLOWMUX_WORKSPACE_ID"),
            Some(ws.to_string().as_str())
        );
        assert_eq!(
            collect(&env, "FLOWMUX_TAB_ID"),
            Some(ws.to_string().as_str())
        );
        assert_eq!(
            collect(&env, "FLOWMUX_SOCKET_PATH"),
            Some("/run/user/1000/flowmux.sock")
        );
        assert!(collect(&env, "FLOWMUX_BUNDLED_CLI_PATH").is_none());
    }

    #[test]
    fn agent_pty_env_emits_pane_and_surface_alias_with_same_value() {
        let pane = PaneId::new();
        let ws = WorkspaceId::new();
        let env = agent_pty_env(pane, ws, Path::new("/x"), None);
        assert_eq!(
            collect(&env, "FLOWMUX_PANE_ID"),
            collect(&env, "FLOWMUX_SURFACE_ID"),
            "cmux convention: panel and surface ids alias to the same value"
        );
        assert_eq!(
            collect(&env, "FLOWMUX_WORKSPACE_ID"),
            collect(&env, "FLOWMUX_TAB_ID"),
            "cmux convention: tab and workspace ids alias"
        );
    }

    #[test]
    fn agent_pty_env_includes_bundled_cli_when_provided() {
        let env = agent_pty_env(
            PaneId::new(),
            WorkspaceId::new(),
            Path::new("/sock"),
            Some(Path::new("/usr/local/bin/flowmux")),
        );
        assert_eq!(
            collect(&env, "FLOWMUX_BUNDLED_CLI_PATH"),
            Some("/usr/local/bin/flowmux")
        );
    }

    #[test]
    fn env_to_kv_strings_joins_pairs_with_equals() {
        let env = vec![
            ("A".into(), "1".into()),
            ("FLOWMUX_PANE_ID".into(), "abc".into()),
        ];
        let kv = env_to_kv_strings(&env);
        assert_eq!(kv, vec!["A=1".to_string(), "FLOWMUX_PANE_ID=abc".to_string()]);
    }

    /// Scenario: building the env we will pass to `vte_terminal_spawn_async`.
    /// Verifies the full pipeline (`agent_pty_env` → `env_to_kv_strings`)
    /// produces a valid envv array as VTE expects.
    #[test]
    fn scenario_full_envv_array_is_well_formed_for_vte_spawn() {
        let pane = PaneId::new();
        let ws = WorkspaceId::new();
        let env = agent_pty_env(
            pane,
            ws,
            Path::new("/run/user/1000/flowmux.sock"),
            Some(Path::new("/usr/local/bin/flowmux")),
        );
        let kv = env_to_kv_strings(&env);

        assert_eq!(kv.len(), 6);
        for entry in &kv {
            let eq = entry.find('=').expect("envv entry must have '='");
            let key = &entry[..eq];
            let val = &entry[eq + 1..];
            assert!(!key.is_empty(), "envv key must be non-empty");
            assert!(key.starts_with("FLOWMUX_"), "expected FLOWMUX_ prefix in {entry}");
            assert!(!val.is_empty(), "envv value must be non-empty");
        }

        let pane_kv = format!("FLOWMUX_PANE_ID={pane}");
        assert!(kv.iter().any(|e| e == &pane_kv));
    }
}
