//! Main application window. Composes header bar + sidebar + content
//! stack and exposes a [`WindowController`] that routes [`GtkCommand`]
//! values from the bridge to widget operations.

use crate::bridge::GtkCommand;
use crate::ui::sidebar::Sidebar;
use crate::ui::terminal_pane::PaneCallbacks;
use crate::ui::workspace_view::{build_surface, PaneRegistry};
use adw::prelude::*;
use flowmux_core::{Pane, PaneContent, PaneId, Surface, SurfaceId, SurfaceKind, Workspace, WorkspaceId};
use flowmux_daemon::StateStore;
use gtk::glib;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone)]
pub struct WindowController {
    pub window: adw::ApplicationWindow,
    sidebar: Sidebar,
    stack: gtk::Stack,
    surfaces: Rc<RefCell<HashMap<WorkspaceId, gtk::Widget>>>,
    pane_registry: Rc<RefCell<PaneRegistry>>,
    callbacks: PaneCallbacks,
    store: StateStore,
}

impl WindowController {
    pub fn new(app: &adw::Application, store: StateStore) -> Self {
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_hexpand(true);
        stack.set_vexpand(true);

        let surfaces: Rc<RefCell<HashMap<WorkspaceId, gtk::Widget>>> =
            Rc::new(RefCell::new(HashMap::new()));
        let surfaces_for_select = surfaces.clone();
        let stack_for_select = stack.clone();

        let sidebar = Sidebar::new(move |id| {
            if surfaces_for_select.borrow().contains_key(&id) {
                stack_for_select.set_visible_child_name(&id.to_string());
            }
        });

        let pane_registry: Rc<RefCell<PaneRegistry>> = Rc::new(RefCell::new(PaneRegistry::default()));
        let callbacks = make_callbacks();

        let split = adw::OverlaySplitView::builder()
            .min_sidebar_width(240.0)
            .max_sidebar_width(360.0)
            .show_sidebar(true)
            .sidebar(&sidebar.root)
            .content(&stack)
            .build();

        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&split));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(1280)
            .default_height(800)
            .title("flowmux")
            .build();
        window.set_content(Some(&toolbar));

        Self { window, sidebar, stack, surfaces, pane_registry, callbacks, store }
    }

    pub fn show_status_when_empty(&self) {
        if self.surfaces.borrow().is_empty() {
            let status = adw::StatusPage::builder()
                .icon_name("utilities-terminal-symbolic")
                .title("flowmux")
                .description(
                    "No workspaces yet — open one with: flowmux workspace new --root .",
                )
                .build();
            self.stack.add_named(&status, Some("__empty"));
            self.stack.set_visible_child_name("__empty");
        }
    }

    pub fn render_workspace(&self, ws: &Workspace) {
        self.sidebar.upsert(ws);
        let mut surfaces = self.surfaces.borrow_mut();
        if surfaces.contains_key(&ws.id) {
            return;
        }
        let widget = self.build_workspace_widget(ws);
        let name = ws.id.to_string();
        self.stack.add_named(&widget, Some(&name));
        surfaces.insert(ws.id, widget);
        self.stack.set_visible_child_name(&name);
    }

    pub fn rerender_workspace(&self, ws: &Workspace) {
        self.sidebar.upsert(ws);
        let name = ws.id.to_string();
        let new_widget = self.build_workspace_widget(ws);
        let mut surfaces = self.surfaces.borrow_mut();
        if let Some(old) = surfaces.remove(&ws.id) {
            self.stack.remove(&old);
        }
        self.stack.add_named(&new_widget, Some(&name));
        surfaces.insert(ws.id, new_widget);
        self.stack.set_visible_child_name(&name);
    }

    fn build_workspace_widget(&self, ws: &Workspace) -> gtk::Widget {
        match ws.surfaces.first() {
            Some(s) => build_surface(s, &self.callbacks, self.pane_registry.clone()),
            None => gtk::Label::new(Some("(empty workspace)")).upcast(),
        }
    }

    pub async fn dispatch(&self, cmd: GtkCommand) {
        match cmd {
            GtkCommand::WorkspaceCreated { id, name, root, ack } => {
                let ws = Workspace {
                    id,
                    name,
                    root_dir: root.clone(),
                    git: None,
                    listening_ports: vec![],
                    surfaces: vec![Surface {
                        id: SurfaceId::new(),
                        kind: SurfaceKind::Terminal {
                            shell: None,
                            cwd: Some(root),
                        },
                        title: "main".into(),
                        root_pane: Pane::Leaf {
                            id: PaneId::new(),
                            content: PaneContent::Terminal { pid: None },
                        },
                    }],
                };
                self.render_workspace(&ws);
                let _ = ack.send(());
            }
            GtkCommand::WorkspaceRerender { id, ack } => {
                if let Some(ws) = self.store.get_workspace(id).await {
                    self.rerender_workspace(&ws);
                }
                let _ = ack.send(());
            }
            GtkCommand::PaneSendKeys { pane, keys, ack } => {
                let registry = self.pane_registry.borrow();
                let res = match registry.terminals.get(&pane) {
                    Some(p) => {
                        p.feed(keys.as_bytes());
                        Ok(())
                    }
                    None => Err(format!("pane not found: {pane}")),
                };
                let _ = ack.send(res);
            }
            GtkCommand::NotificationOnPane { pane, title, body } => {
                tracing::info!(%pane, %title, %body, "pane notification");
                // TODO: paint blue ring + sidebar badge.
            }
            GtkCommand::InjectCookies { cookies, ack } => {
                let result = inject_cookies_into_webkit(&cookies);
                let _ = ack.send(result);
            }
            GtkCommand::BrowserEval { pane, source, ack } => {
                let registry = self.pane_registry.borrow();
                match registry.browsers.get(&pane) {
                    None => {
                        let _ = ack.send(Err(format!("browser pane not found: {pane}")));
                    }
                    Some(browser) => {
                        // evaluate_js is callback-style; bridge it to the ack.
                        let cell = std::cell::Cell::new(Some(ack));
                        browser.evaluate_js(&source, move |result| {
                            if let Some(ack) = cell.take() {
                                let _ = ack.send(result);
                            }
                        });
                    }
                }
            }
        }
    }

    pub async fn restore_from_store(&self) {
        let snap = self.store.snapshot().await;
        for ws in &snap.workspaces {
            self.render_workspace(ws);
        }
        if let Some(active) = snap.active_workspace {
            if self.surfaces.borrow().contains_key(&active) {
                self.stack.set_visible_child_name(&active.to_string());
            }
        }
    }
}

fn make_callbacks() -> PaneCallbacks {
    use std::cell::RefCell;
    use std::rc::Rc;
    PaneCallbacks {
        on_notification: Rc::new(RefCell::new(|pane, title, body| {
            tracing::info!(%pane, %title, %body, "OSC 99 from pane");
            // The full path (forward to daemon → desktop notifier) is
            // wired in main.rs via a glib mainloop bridge.
        })),
        on_bell: Rc::new(RefCell::new(|pane| {
            tracing::debug!(%pane, "BEL");
        })),
        on_child_exited: Rc::new(RefCell::new(|pane, status| {
            tracing::info!(%pane, status, "child exited");
        })),
    }
}

/// Inject cookies into the default WebKit network session.
///
/// Real injection goes through `WebKit.NetworkSession.cookie_manager()`
/// → `CookieManager.add_cookie(&soup::Cookie, ...)`. The `soup::Cookie`
/// type is only re-exported from webkit6 when the `soup3` feature is
/// enabled (which in turn pulls in libsoup-3). To keep the default
/// build minimal we record the cookies that *would* be injected and
/// return the count; flipping `flowmux-app/Cargo.toml` to
/// `webkit6 = { version = "0.4", features = ["soup3"] }` and replacing
/// the body below with `manager.add_cookie(...)` calls is the only
/// change needed when we ship cookie import to users.
fn inject_cookies_into_webkit(
    cookies: &[flowmux_cookies::Cookie],
) -> Result<usize, String> {
    let mut count = 0;
    for c in cookies {
        tracing::debug!(host = %c.host, name = %c.name, "would inject cookie");
        count += 1;
    }
    Ok(count)
}

/// Spawn the GTK-side dispatch loop. Lives on the main context.
pub fn spawn_dispatch_loop(
    rx: async_channel::Receiver<GtkCommand>,
    controller: WindowController,
) {
    glib::MainContext::default().spawn_local(async move {
        while let Ok(cmd) = rx.recv().await {
            controller.dispatch(cmd).await;
        }
    });
}
