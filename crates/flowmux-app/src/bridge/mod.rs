//! Bridge between the tokio IPC handler thread and the GTK main loop.
//!
//! GTK widgets are `!Send`, so anything that touches the widget tree
//! must run on the main thread. The IPC server, state store, and
//! desktop notifier all run on tokio. We connect them with an async
//! channel: tokio side sends [`GtkCommand`] values, the GTK side reads
//! them via `glib::MainContext::spawn_local` and dispatches into the
//! window controller.

use flowmux_core::{PaneId, WorkspaceId};
use std::path::PathBuf;
use tokio::sync::oneshot;

/// One-way commands from tokio → GTK main loop. Each variant carries a
/// `oneshot::Sender` for replies if the caller needs the result.
#[derive(Debug)]
pub enum GtkCommand {
    /// Render a freshly-created workspace in the sidebar + open its first pane.
    WorkspaceCreated {
        id: WorkspaceId,
        name: String,
        root: PathBuf,
        ack: oneshot::Sender<()>,
    },
    /// Re-render a workspace from the latest store snapshot.
    /// Used after structural mutations like split.
    WorkspaceRerender {
        id: WorkspaceId,
        ack: oneshot::Sender<()>,
    },
    /// Send keystrokes to a pane.
    PaneSendKeys {
        pane: PaneId,
        keys: String,
        ack: oneshot::Sender<Result<(), String>>,
    },
    /// A notification was raised on a pane (from VTE OSC signal). Update
    /// the pane border / sidebar badge.
    #[allow(dead_code)]
    NotificationOnPane {
        pane: PaneId,
        title: String,
        body: String,
    },
    /// Evaluate JavaScript in a browser pane.
    BrowserEval {
        pane: PaneId,
        source: String,
        ack: oneshot::Sender<Result<String, String>>,
    },
    /// Inject a list of cookies into the WebKit cookie manager.
    InjectCookies {
        cookies: Vec<flowmux_cookies::Cookie>,
        ack: oneshot::Sender<Result<usize, String>>,
    },
}

#[derive(Clone)]
pub struct Bridge {
    pub tx: async_channel::Sender<GtkCommand>,
}

impl Bridge {
    pub fn new() -> (Self, async_channel::Receiver<GtkCommand>) {
        let (tx, rx) = async_channel::unbounded();
        (Self { tx }, rx)
    }
}
