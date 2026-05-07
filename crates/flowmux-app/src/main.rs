//! flowmux GUI entrypoint.
//!
//! Boots libadwaita, brings up the IPC server and the state store on a
//! tokio runtime, and wires GTK-affecting verbs to the GTK main loop
//! through an [`async_channel`] bridge.

mod bridge;
mod ipc_handler;
mod ui;

use adw::prelude::*;
use bridge::Bridge;
use flowmux_config::paths;
use flowmux_daemon::{DaemonHandler, StateStore};
use ipc_handler::GuiHandler;
use std::sync::Arc;
use tracing::info;
use ui::{spawn_dispatch_loop, WindowController};

const APP_ID: &str = "com.flowmux.App";

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("FLOWMUX_LOG")
                .unwrap_or_else(|_| "info,flowmux=debug".into()),
        )
        .init();

    let socket = paths::runtime_socket();
    info!(?socket, "flowmux-app starting");

    // Tokio runtime hosts the IPC server, the state store, and any
    // async desktop-bus interactions.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let initial = match flowmux_state::load() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "could not load state, starting empty");
            flowmux_state::State::default()
        }
    };
    let store = StateStore::new_lazy(initial);
    store.spawn_persist(rt.handle());

    let (bridge, rx) = Bridge::new();
    let handler = Arc::new(GuiHandler::new(
        DaemonHandler::new(store.clone()),
        bridge.clone(),
    ));

    let socket_clone = socket.clone();
    rt.spawn(async move {
        if let Err(e) = flowmux_ipc::server::run(&socket_clone, handler).await {
            tracing::error!(error = %e, "ipc server exited");
        }
    });

    // GTK runs on the main thread.
    let app = adw::Application::builder().application_id(APP_ID).build();
    let store_for_activate = store.clone();
    let rx_for_activate = rx.clone();
    app.connect_activate(move |app| {
        let controller = WindowController::new(app, store_for_activate.clone());
        spawn_dispatch_loop(rx_for_activate.clone(), controller.clone());
        let controller_for_init = controller.clone();
        gtk::glib::MainContext::default().spawn_local(async move {
            controller_for_init.restore_from_store().await;
            controller_for_init.show_status_when_empty();
        });
        controller.window.present();
    });

    let exit_code = app.run();
    drop(rt);
    let _ = std::fs::remove_file(&socket);
    std::process::exit(exit_code.into());
}
