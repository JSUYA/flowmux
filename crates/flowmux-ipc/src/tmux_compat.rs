// SPDX-License-Identifier: GPL-3.0-or-later
//! tmux CLI compatibility layer for Claude Code agent teams.
//!
//! Claude Code's "agent teams" split-pane display drives a `tmux`
//! binary found on `PATH`. flowmux installs a `tmux` shim into the
//! agent shim dir (prepended to every pane's `PATH`) that forwards
//! swarm-scoped invocations to `flowmuxctl tmux-compat`, which sends a
//! [`crate::protocol::Request::TmuxCompat`] to the daemon. This module
//! is the pure argv → command parser plus the small shared vocabulary
//! (session-key mapping, `#{...}` format expansion) both daemon
//! implementations use. It has no I/O so it is fully unit testable.
//!
//! Only the subset of tmux actually invoked by Claude Code's teammate
//! backend is modeled (observed from Claude Code v2.1.207):
//!
//! ```text
//! tmux -V
//! tmux -L claude-swarm-<pid> has-session -t claude-swarm
//! tmux -L ... new-session -d -s claude-swarm -n swarm-view -P -F '#{pane_id}' -- cat
//! tmux -L ... list-windows -t claude-swarm -F '#{window_name}'
//! tmux -L ... new-window -t claude-swarm -n swarm-view -P -F '#{pane_id}' -- cat
//! tmux -L ... list-panes -t claude-swarm:swarm-view -F '#{pane_id}'
//! tmux -L ... split-window -d -t <pane> -v|-h [-l 70%] -P -F '#{pane_id}' -- cat
//! tmux -L ... set-option -p -t <pane> remain-on-exit failed        (and styles)
//! tmux -L ... set-option -w -t <target> pane-border-status top
//! tmux -L ... select-pane -t <pane> -T <title>
//! tmux -L ... respawn-pane -k -t <pane> -- '<shell command>'
//! tmux -L ... kill-pane -t <pane>
//! tmux -L ... select-layout -t <target> tiled
//! ```
//!
//! An older teammate path uses the default server socket and one
//! window per teammate (`new-window -t claude-swarm -n teammate-<x>`),
//! so targets are honored with or without `-L`.
//!
//! Mapping to flowmux: a tmux *session* (plus its server socket name)
//! becomes one workspace; every tmux *pane* is a flowmux pane. Pane
//! ids are opaque strings to Claude Code — it only echoes them back in
//! `-t` — so flowmux returns its own pane UUIDs instead of `%N`
//! numbers and never needs an id-mapping table.

use serde::{Deserialize, Serialize};

/// Version string reported for `tmux -V`. Claude Code only checks the
/// exit status, but a human running the shim should see what it is.
pub const SHIM_VERSION_LINE: &str = "tmux 3.4 (flowmux tmux-compat shim)";

/// A pane target: either a flowmux pane UUID we handed out earlier, or
/// a `session[:window]` name target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `-t <uuid>` — a pane id previously returned by this layer.
    Pane(String),
    /// `-t <session>` or `-t <session>:<window>`.
    Session {
        session: String,
        window: Option<String>,
    },
}

impl Target {
    /// Parse a `-t` argument. UUIDs are how we hand out pane ids;
    /// anything else is a session (optionally `session:window`) name.
    pub fn parse(raw: &str) -> Target {
        // Real tmux pane ids look like `%3`; ours are UUIDs. Accept
        // both shapes as pane targets so logs stay debuggable.
        let looks_like_uuid =
            raw.len() == 36 && raw.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        if looks_like_uuid || raw.starts_with('%') {
            return Target::Pane(raw.trim_start_matches('%').to_string());
        }
        match raw.split_once(':') {
            Some((session, window)) => Target::Session {
                session: session.to_string(),
                window: (!window.is_empty()).then(|| window.to_string()),
            },
            None => Target::Session {
                session: raw.to_string(),
                window: None,
            },
        }
    }
}

/// One parsed tmux invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxInvocation {
    /// `-L <name>` server socket name, when given. Distinguishes
    /// concurrent Claude Code leads (each uses `claude-swarm-<pid>`).
    pub socket_name: Option<String>,
    pub command: TmuxCommand,
}

/// The tmux subcommands Claude Code's teammate backend issues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TmuxCommand {
    /// `tmux -V`
    Version,
    /// `has-session -t <session>` — exit 0 iff it exists.
    HasSession { target: Target },
    /// `new-session -d -s <name> [-n <window>] [-c dir] [-P -F fmt] [cmd]`
    NewSession {
        name: String,
        window_name: Option<String>,
        print: bool,
        format: Option<String>,
    },
    /// `list-windows -t <target> -F <fmt>`
    ListWindows {
        target: Target,
        format: Option<String>,
    },
    /// `new-window -t <session> [-n <name>] [-P -F fmt] [cmd]`
    NewWindow {
        target: Target,
        window_name: Option<String>,
        print: bool,
        format: Option<String>,
    },
    /// `list-panes -t <target> -F <fmt>`
    ListPanes {
        target: Target,
        format: Option<String>,
    },
    /// `split-window [-d] -t <pane> [-h|-v] [-l size] [-P -F fmt] [cmd]`
    SplitWindow {
        target: Target,
        /// tmux `-h` = new pane to the right (side by side).
        horizontal: bool,
        print: bool,
        format: Option<String>,
    },
    /// `set-option [-p|-w] -t <target> <name> <value...>` — cosmetic
    /// (border styles / formats / remain-on-exit); acknowledged, not acted on.
    SetOption {
        target: Option<Target>,
        name: String,
    },
    /// `select-pane -t <pane> [-T <title>]`
    SelectPane {
        target: Target,
        title: Option<String>,
    },
    /// `respawn-pane [-k] -t <pane> -- <shell command>` — run the
    /// teammate command inside the pane.
    RespawnPane { target: Target, command: String },
    /// `kill-pane -t <pane>`
    KillPane { target: Target },
    /// `select-layout -t <target> <layout>` — acknowledged, not acted on
    /// (flowmux keeps its own split ratios).
    SelectLayout { target: Target },
    /// `display-message -p <fmt>` — best-effort; used by Claude Code
    /// only when it believes it runs inside tmux.
    DisplayMessage { format: Option<String> },
    /// `attach` / `attach-session [-t <session>]` — focus the mapped
    /// workspace instead of attaching a client.
    Attach { target: Option<Target> },
    /// `kill-server` — drop every workspace mapped to this socket name.
    KillServer,
}

/// Parse error carrying the message the shim prints to stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseError {}

/// Derive the workspace name a (socket, session) pair maps to. The
/// socket name already embeds the lead's pid (`claude-swarm-<pid>`),
/// so it alone keeps concurrent teams apart; default-socket sessions
/// fall back to the bare session name.
pub fn session_workspace_name(socket_name: Option<&str>, session: &str) -> String {
    match socket_name {
        Some(sock) => sock.to_string(),
        None => session.to_string(),
    }
}

/// Expand the tiny `#{...}` format subset Claude Code uses. Unknown
/// tokens expand to the empty string like tmux does for unset ones.
pub fn expand_format(format: &str, pane_id: &str, window_name: &str, session_name: &str) -> String {
    format
        .replace("#{pane_id}", pane_id)
        .replace("#{window_name}", window_name)
        .replace("#{session_name}", session_name)
}

fn missing(flag: &str, cmd: &str) -> ParseError {
    ParseError(format!("tmux-compat: {cmd}: missing value for {flag}"))
}

/// Split argv into (flag map handled per subcommand, trailing command
/// after `--` or first bare word). tmux allows the shell command as
/// trailing words with or without `--`.
struct Args<'a> {
    rest: &'a [String],
    idx: usize,
}

impl<'a> Args<'a> {
    fn new(rest: &'a [String]) -> Self {
        Self { rest, idx: 0 }
    }

    fn next(&mut self) -> Option<&'a str> {
        let v = self.rest.get(self.idx).map(String::as_str);
        if v.is_some() {
            self.idx += 1;
        }
        v
    }

    /// Remaining args joined as the trailing shell command.
    fn trailing_command(&mut self) -> Option<String> {
        let words = &self.rest[self.idx..];
        self.idx = self.rest.len();
        if words.is_empty() {
            None
        } else {
            Some(words.join(" "))
        }
    }
}

/// Parse a full tmux argv (without the leading `tmux`).
pub fn parse(args: &[String]) -> Result<TmuxInvocation, ParseError> {
    let mut socket_name: Option<String> = None;
    let mut it = Args::new(args);

    // Global flags come before the subcommand.
    let sub = loop {
        match it.next() {
            None => {
                // Bare `tmux` — Claude Code never does this; the shim
                // passes it through to real tmux before we get here.
                return Err(ParseError(
                    "tmux-compat: no subcommand (interactive tmux is not emulated)".into(),
                ));
            }
            Some("-V") => break "-V",
            Some("-L") => {
                socket_name = Some(it.next().ok_or_else(|| missing("-L", "tmux"))?.to_string());
            }
            Some("-S") => {
                // A socket *path* — derive a stable name from the file
                // stem so `-S /tmp/x/claude-swarm-1.sock` still groups.
                let path = it.next().ok_or_else(|| missing("-S", "tmux"))?;
                let stem = std::path::Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string());
                socket_name = Some(stem);
            }
            Some("-f") => {
                let _ = it.next(); // config file — irrelevant here.
            }
            Some(word) if !word.starts_with('-') => break word,
            Some(other) => {
                return Err(ParseError(format!(
                    "tmux-compat: unsupported global flag {other}"
                )));
            }
        }
    };

    let command = match sub {
        "-V" => TmuxCommand::Version,
        "has-session" => {
            let target = parse_target_only(&mut it, "has-session")?;
            TmuxCommand::HasSession { target }
        }
        "new-session" | "new" => {
            let mut name = None;
            let mut window_name = None;
            let mut print = false;
            let mut format = None;
            loop {
                match it.next() {
                    None => break,
                    Some("-d") | Some("-A") => {}
                    Some("-s") => {
                        name = Some(it.next().ok_or_else(|| missing("-s", sub))?.to_string())
                    }
                    Some("-n") => {
                        window_name = Some(it.next().ok_or_else(|| missing("-n", sub))?.to_string())
                    }
                    Some("-c") => {
                        let _ = it.next(); // start dir: workspace root comes from the request cwd.
                    }
                    Some("-P") => print = true,
                    Some("-F") => {
                        format = Some(it.next().ok_or_else(|| missing("-F", sub))?.to_string())
                    }
                    Some("--") | Some(_) => {
                        // Placeholder command (`cat`) — the pane runs
                        // the user shell until respawn-pane arrives.
                        let _ = it.trailing_command();
                        break;
                    }
                }
            }
            TmuxCommand::NewSession {
                name: name
                    .ok_or_else(|| ParseError("tmux-compat: new-session requires -s".into()))?,
                window_name,
                print,
                format,
            }
        }
        "list-windows" | "lsw" => {
            let (target, format) = parse_target_format(&mut it, sub)?;
            TmuxCommand::ListWindows {
                target: target
                    .ok_or_else(|| ParseError("tmux-compat: list-windows requires -t".into()))?,
                format,
            }
        }
        "new-window" | "neww" => {
            let mut target = None;
            let mut window_name = None;
            let mut print = false;
            let mut format = None;
            loop {
                match it.next() {
                    None => break,
                    Some("-d") => {}
                    Some("-t") => {
                        target = Some(Target::parse(it.next().ok_or_else(|| missing("-t", sub))?))
                    }
                    Some("-n") => {
                        window_name = Some(it.next().ok_or_else(|| missing("-n", sub))?.to_string())
                    }
                    Some("-P") => print = true,
                    Some("-F") => {
                        format = Some(it.next().ok_or_else(|| missing("-F", sub))?.to_string())
                    }
                    Some("--") | Some(_) => {
                        let _ = it.trailing_command();
                        break;
                    }
                }
            }
            TmuxCommand::NewWindow {
                target: target
                    .ok_or_else(|| ParseError("tmux-compat: new-window requires -t".into()))?,
                window_name,
                print,
                format,
            }
        }
        "list-panes" | "lsp" => {
            let (target, format) = parse_target_format(&mut it, sub)?;
            TmuxCommand::ListPanes {
                target: target
                    .ok_or_else(|| ParseError("tmux-compat: list-panes requires -t".into()))?,
                format,
            }
        }
        "split-window" | "splitw" => {
            let mut target = None;
            let mut horizontal = false;
            let mut print = false;
            let mut format = None;
            loop {
                match it.next() {
                    None => break,
                    Some("-d") => {}
                    Some("-h") => horizontal = true,
                    Some("-v") => horizontal = false,
                    Some("-l") | Some("-p") => {
                        let _ = it.next(); // size hint — flowmux splits 50/50.
                    }
                    Some("-t") => {
                        target = Some(Target::parse(it.next().ok_or_else(|| missing("-t", sub))?))
                    }
                    Some("-P") => print = true,
                    Some("-F") => {
                        format = Some(it.next().ok_or_else(|| missing("-F", sub))?.to_string())
                    }
                    Some("--") | Some(_) => {
                        let _ = it.trailing_command();
                        break;
                    }
                }
            }
            TmuxCommand::SplitWindow {
                target: target
                    .ok_or_else(|| ParseError("tmux-compat: split-window requires -t".into()))?,
                horizontal,
                print,
                format,
            }
        }
        "set-option" | "set" => {
            let mut target = None;
            let mut name = None;
            loop {
                match it.next() {
                    None => break,
                    Some("-p") | Some("-w") | Some("-g") | Some("-q") => {}
                    Some("-t") => {
                        target = Some(Target::parse(it.next().ok_or_else(|| missing("-t", sub))?))
                    }
                    Some(word) => {
                        name = Some(word.to_string());
                        let _ = it.trailing_command(); // option value(s)
                        break;
                    }
                }
            }
            TmuxCommand::SetOption {
                target,
                name: name
                    .ok_or_else(|| ParseError("tmux-compat: set-option requires a name".into()))?,
            }
        }
        "select-pane" | "selectp" => {
            let mut target = None;
            let mut title = None;
            loop {
                match it.next() {
                    None => break,
                    Some("-t") => {
                        target = Some(Target::parse(it.next().ok_or_else(|| missing("-t", sub))?))
                    }
                    Some("-T") => {
                        title = Some(it.next().ok_or_else(|| missing("-T", sub))?.to_string())
                    }
                    Some(_) => {}
                }
            }
            TmuxCommand::SelectPane {
                target: target
                    .ok_or_else(|| ParseError("tmux-compat: select-pane requires -t".into()))?,
                title,
            }
        }
        "respawn-pane" | "respawnp" => {
            let mut target = None;
            let mut command = None;
            loop {
                match it.next() {
                    None => break,
                    Some("-k") => {}
                    Some("-t") => {
                        target = Some(Target::parse(it.next().ok_or_else(|| missing("-t", sub))?))
                    }
                    Some("--") => {
                        command = it.trailing_command();
                        break;
                    }
                    Some(word) => {
                        let mut cmd = word.to_string();
                        if let Some(rest) = it.trailing_command() {
                            cmd.push(' ');
                            cmd.push_str(&rest);
                        }
                        command = Some(cmd);
                        break;
                    }
                }
            }
            TmuxCommand::RespawnPane {
                target: target
                    .ok_or_else(|| ParseError("tmux-compat: respawn-pane requires -t".into()))?,
                command: command.ok_or_else(|| {
                    ParseError("tmux-compat: respawn-pane requires a command".into())
                })?,
            }
        }
        "kill-pane" | "killp" => {
            let target = parse_target_only(&mut it, "kill-pane")?;
            TmuxCommand::KillPane { target }
        }
        "select-layout" | "selectl" => {
            let mut target = None;
            loop {
                match it.next() {
                    None => break,
                    Some("-t") => {
                        target = Some(Target::parse(it.next().ok_or_else(|| missing("-t", sub))?))
                    }
                    Some(_) => {} // layout name — acknowledged only.
                }
            }
            TmuxCommand::SelectLayout {
                target: target
                    .ok_or_else(|| ParseError("tmux-compat: select-layout requires -t".into()))?,
            }
        }
        "display-message" | "display" => {
            let mut format = None;
            loop {
                match it.next() {
                    None => break,
                    Some("-p") => {}
                    Some("-t") => {
                        let _ = it.next();
                    }
                    Some(word) => {
                        format = Some(word.to_string());
                        break;
                    }
                }
            }
            TmuxCommand::DisplayMessage { format }
        }
        "attach" | "attach-session" | "a" | "at" => {
            let mut target = None;
            loop {
                match it.next() {
                    None => break,
                    Some("-t") => {
                        target = Some(Target::parse(it.next().ok_or_else(|| missing("-t", sub))?))
                    }
                    Some(_) => {}
                }
            }
            TmuxCommand::Attach { target }
        }
        "kill-server" => TmuxCommand::KillServer,
        other => {
            return Err(ParseError(format!(
                "tmux-compat: unsupported tmux subcommand '{other}'"
            )));
        }
    };

    Ok(TmuxInvocation {
        socket_name,
        command,
    })
}

fn parse_target_only(it: &mut Args<'_>, cmd: &str) -> Result<Target, ParseError> {
    let mut target = None;
    loop {
        match it.next() {
            None => break,
            Some("-t") => {
                target = Some(Target::parse(it.next().ok_or_else(|| missing("-t", cmd))?))
            }
            Some(_) => {}
        }
    }
    target.ok_or_else(|| ParseError(format!("tmux-compat: {cmd} requires -t")))
}

fn parse_target_format(
    it: &mut Args<'_>,
    cmd: &str,
) -> Result<(Option<Target>, Option<String>), ParseError> {
    let mut target = None;
    let mut format = None;
    loop {
        match it.next() {
            None => break,
            Some("-t") => {
                target = Some(Target::parse(it.next().ok_or_else(|| missing("-t", cmd))?))
            }
            Some("-F") => format = Some(it.next().ok_or_else(|| missing("-F", cmd))?.to_string()),
            Some("-a") | Some("-s") => {}
            Some(_) => {}
        }
    }
    Ok((target, format))
}

/// The wire result the daemon returns to the shim: mirrors a process
/// exit (status / stdout / stderr) so the shim can reproduce it 1:1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TmuxCompatOutput {
    pub code: i32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stdout: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr: String,
}

impl TmuxCompatOutput {
    pub fn ok() -> Self {
        Self {
            code: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    pub fn out(stdout: impl Into<String>) -> Self {
        Self {
            code: 0,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub fn fail(code: i32, stderr: impl Into<String>) -> Self {
        Self {
            code,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|w| w.to_string()).collect()
    }

    // ---- exact invocations observed from Claude Code v2.1.207 ------

    #[test]
    fn version_probe_parses() {
        let inv = parse(&args(&["-V"])).unwrap();
        assert_eq!(inv.command, TmuxCommand::Version);
        assert_eq!(inv.socket_name, None);
    }

    #[test]
    fn has_session_with_swarm_socket() {
        let inv = parse(&args(&[
            "-L",
            "claude-swarm-12345",
            "has-session",
            "-t",
            "claude-swarm",
        ]))
        .unwrap();
        assert_eq!(inv.socket_name.as_deref(), Some("claude-swarm-12345"));
        assert_eq!(
            inv.command,
            TmuxCommand::HasSession {
                target: Target::Session {
                    session: "claude-swarm".into(),
                    window: None
                }
            }
        );
    }

    #[test]
    fn new_session_external_swarm_shape() {
        // iZi.createExternalSwarmSession: new-session -d -s claude-swarm
        //   -n swarm-view -P -F '#{pane_id}' -- cat
        let inv = parse(&args(&[
            "-L",
            "claude-swarm-777",
            "new-session",
            "-d",
            "-s",
            "claude-swarm",
            "-n",
            "swarm-view",
            "-P",
            "-F",
            "#{pane_id}",
            "--",
            "cat",
        ]))
        .unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::NewSession {
                name: "claude-swarm".into(),
                window_name: Some("swarm-view".into()),
                print: true,
                format: Some("#{pane_id}".into()),
            }
        );
    }

    #[test]
    fn new_session_plain_with_start_dir() {
        // M5i (worktree helper): new-session -d -s <name> -c <dir>
        let inv = parse(&args(&[
            "new-session",
            "-d",
            "-s",
            "mywork",
            "-c",
            "/tmp/x",
        ]))
        .unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::NewSession {
                name: "mywork".into(),
                window_name: None,
                print: false,
                format: None,
            }
        );
    }

    #[test]
    fn list_windows_shape() {
        let inv = parse(&args(&[
            "-L",
            "claude-swarm-1",
            "list-windows",
            "-t",
            "claude-swarm",
            "-F",
            "#{window_name}",
        ]))
        .unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::ListWindows {
                target: Target::Session {
                    session: "claude-swarm".into(),
                    window: None
                },
                format: Some("#{window_name}".into()),
            }
        );
    }

    #[test]
    fn new_window_swarm_view_and_legacy_teammate() {
        let inv = parse(&args(&[
            "-L",
            "claude-swarm-1",
            "new-window",
            "-t",
            "claude-swarm",
            "-n",
            "swarm-view",
            "-P",
            "-F",
            "#{pane_id}",
            "--",
            "cat",
        ]))
        .unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::NewWindow {
                target: Target::Session {
                    session: "claude-swarm".into(),
                    window: None
                },
                window_name: Some("swarm-view".into()),
                print: true,
                format: Some("#{pane_id}".into()),
            }
        );

        // Legacy path: default socket, one window per teammate.
        let inv = parse(&args(&[
            "new-window",
            "-t",
            "claude-swarm",
            "-n",
            "teammate-researcher",
            "-P",
            "-F",
            "#{pane_id}",
            "--",
            "cat",
        ]))
        .unwrap();
        assert_eq!(inv.socket_name, None);
        match inv.command {
            TmuxCommand::NewWindow { window_name, .. } => {
                assert_eq!(window_name.as_deref(), Some("teammate-researcher"));
            }
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn list_panes_by_session_window_and_by_pane() {
        let inv = parse(&args(&[
            "-L",
            "claude-swarm-1",
            "list-panes",
            "-t",
            "claude-swarm:swarm-view",
            "-F",
            "#{pane_id}",
        ]))
        .unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::ListPanes {
                target: Target::Session {
                    session: "claude-swarm".into(),
                    window: Some("swarm-view".into())
                },
                format: Some("#{pane_id}".into()),
            }
        );

        // Leader path targets a pane id directly.
        let uuid = "0b8e7f66-90bc-4f74-9e2e-7f3f4be2a111";
        let inv = parse(&args(&["list-panes", "-t", uuid, "-F", "#{pane_id}"])).unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::ListPanes {
                target: Target::Pane(uuid.into()),
                format: Some("#{pane_id}".into()),
            }
        );
    }

    #[test]
    fn split_window_directions_and_size() {
        let uuid = "0b8e7f66-90bc-4f74-9e2e-7f3f4be2a111";
        // First teammate split in leader mode: -h -l 70%
        let inv = parse(&args(&[
            "split-window",
            "-d",
            "-t",
            uuid,
            "-h",
            "-l",
            "70%",
            "-P",
            "-F",
            "#{pane_id}",
            "--",
            "cat",
        ]))
        .unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::SplitWindow {
                target: Target::Pane(uuid.into()),
                horizontal: true,
                print: true,
                format: Some("#{pane_id}".into()),
            }
        );

        // External swarm split: alternating -v / -h without size.
        let inv = parse(&args(&[
            "-L",
            "claude-swarm-9",
            "split-window",
            "-d",
            "-t",
            uuid,
            "-v",
            "-P",
            "-F",
            "#{pane_id}",
            "--",
            "cat",
        ]))
        .unwrap();
        match inv.command {
            TmuxCommand::SplitWindow { horizontal, .. } => assert!(!horizontal),
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn set_option_remain_on_exit_and_styles() {
        let uuid = "0b8e7f66-90bc-4f74-9e2e-7f3f4be2a111";
        let inv = parse(&args(&[
            "-L",
            "claude-swarm-9",
            "set-option",
            "-p",
            "-t",
            uuid,
            "remain-on-exit",
            "failed",
        ]))
        .unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::SetOption {
                target: Some(Target::Pane(uuid.into())),
                name: "remain-on-exit".into(),
            }
        );

        let inv = parse(&args(&[
            "set-option",
            "-w",
            "-t",
            "claude-swarm:swarm-view",
            "pane-border-status",
            "top",
        ]))
        .unwrap();
        match inv.command {
            TmuxCommand::SetOption { name, .. } => assert_eq!(name, "pane-border-status"),
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn select_pane_title() {
        let uuid = "0b8e7f66-90bc-4f74-9e2e-7f3f4be2a111";
        let inv = parse(&args(&["select-pane", "-t", uuid, "-T", "researcher"])).unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::SelectPane {
                target: Target::Pane(uuid.into()),
                title: Some("researcher".into()),
            }
        );
    }

    #[test]
    fn respawn_pane_with_and_without_double_dash() {
        let uuid = "0b8e7f66-90bc-4f74-9e2e-7f3f4be2a111";
        let teammate_cmd =
            "cd /home/u/proj && env CLAUDECODE=1 CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1 \
             '/usr/bin/claude' --agent-id 'r-1' --agent-name 'researcher' --team-name 'sess-1'";
        let inv = parse(&args(&[
            "respawn-pane",
            "-k",
            "-t",
            uuid,
            "--",
            teammate_cmd,
        ]))
        .unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::RespawnPane {
                target: Target::Pane(uuid.into()),
                command: teammate_cmd.into(),
            }
        );

        let inv = parse(&args(&["respawn-pane", "-k", "-t", uuid, teammate_cmd])).unwrap();
        match inv.command {
            TmuxCommand::RespawnPane { command, .. } => assert_eq!(command, teammate_cmd),
            other => panic!("wrong command: {other:?}"),
        }
    }

    #[test]
    fn kill_pane_and_select_layout_and_attach() {
        let uuid = "0b8e7f66-90bc-4f74-9e2e-7f3f4be2a111";
        let inv = parse(&args(&["kill-pane", "-t", uuid])).unwrap();
        assert_eq!(
            inv.command,
            TmuxCommand::KillPane {
                target: Target::Pane(uuid.into())
            }
        );

        let inv = parse(&args(&[
            "select-layout",
            "-t",
            "claude-swarm:swarm-view",
            "tiled",
        ]))
        .unwrap();
        match inv.command {
            TmuxCommand::SelectLayout { target } => assert_eq!(
                target,
                Target::Session {
                    session: "claude-swarm".into(),
                    window: Some("swarm-view".into())
                }
            ),
            other => panic!("wrong command: {other:?}"),
        }

        // The status-line hint: `tmux -L claude-swarm-<pid> a`
        let inv = parse(&args(&["-L", "claude-swarm-4242", "a"])).unwrap();
        assert_eq!(inv.command, TmuxCommand::Attach { target: None });
    }

    // ---- vocabulary helpers -----------------------------------------

    #[test]
    fn workspace_name_prefers_socket_name() {
        assert_eq!(
            session_workspace_name(Some("claude-swarm-99"), "claude-swarm"),
            "claude-swarm-99"
        );
        assert_eq!(session_workspace_name(None, "claude-swarm"), "claude-swarm");
    }

    #[test]
    fn format_expansion_covers_used_tokens() {
        assert_eq!(
            expand_format("#{pane_id}", "abc-123", "swarm-view", "claude-swarm"),
            "abc-123"
        );
        assert_eq!(
            expand_format("#{window_name}", "x", "swarm-view", "claude-swarm"),
            "swarm-view"
        );
        assert_eq!(
            expand_format("#{session_name}", "x", "w", "claude-swarm"),
            "claude-swarm"
        );
    }

    #[test]
    fn socket_path_derives_stable_name() {
        let inv = parse(&args(&[
            "-S",
            "/tmp/tmux-1000/claude-swarm-5.sock",
            "has-session",
            "-t",
            "claude-swarm",
        ]))
        .unwrap();
        assert_eq!(inv.socket_name.as_deref(), Some("claude-swarm-5"));
    }

    #[test]
    fn unsupported_subcommand_errors() {
        let err = parse(&args(&["pipe-pane", "-t", "%1", "cat"])).unwrap_err();
        assert!(err.0.contains("unsupported tmux subcommand"));
    }

    #[test]
    fn output_shapes_roundtrip() {
        let out = TmuxCompatOutput::out("uuid-1\n");
        let json = serde_json::to_string(&out).unwrap();
        let back: TmuxCompatOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(back, out);
        assert_eq!(TmuxCompatOutput::ok().code, 0);
        assert_eq!(TmuxCompatOutput::fail(1, "no session").code, 1);
    }
}
