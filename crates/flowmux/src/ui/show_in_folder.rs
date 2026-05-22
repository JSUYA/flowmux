// SPDX-License-Identifier: GPL-3.0-or-later
//! Open the system file viewer at a directory path.
//!
//! Used by the sidebar workspace context menu and the pane tab context
//! menu ("Show in folder"). The viewer is whatever `xdg-open` resolves
//! `inode/directory` to (Nautilus on a default Ubuntu/GNOME install).
//!
//! Two execution paths, picked at runtime:
//!
//! - Native build: spawn `xdg-open <dir>` directly.
//! - Flatpak sandbox: spawn `flatpak-spawn --host xdg-open <dir>` so the
//!   file manager runs on the host, not inside the sandbox where it
//!   isn't installed. Detected by the presence of `/.flatpak-info`,
//!   the same marker the rest of the codebase uses for its
//!   `flatpak-spawn --host` shell wrapping. The sandbox already has
//!   `--talk-name=org.freedesktop.Flatpak` per the manifest, so the
//!   spawn call works without portal plumbing.
//!
//! Spawning goes through `gio::Subprocess` so the GLib main loop reaps
//! the child via its built-in child-watch source. A bare
//! `std::process::Command` would leak a zombie until the GUI exits.

use gtk::gio;
use std::path::Path;

/// Open the user's file manager at `dir`. Logs on failure but does not
/// surface an error to the caller — the menu item is best-effort.
pub fn open_directory(dir: &Path) {
    if !dir.is_dir() {
        tracing::warn!(path = %dir.display(), "show-in-folder: path is not a directory");
        return;
    }
    let path_str = dir.to_string_lossy().into_owned();
    let argv: Vec<&str> = if in_flatpak_sandbox() {
        vec!["flatpak-spawn", "--host", "xdg-open", path_str.as_str()]
    } else {
        vec!["xdg-open", path_str.as_str()]
    };
    match gio::Subprocess::newv(
        &argv.iter().map(std::ffi::OsStr::new).collect::<Vec<_>>(),
        gio::SubprocessFlags::NONE,
    ) {
        Ok(_child) => {
            tracing::info!(path = %dir.display(), "show-in-folder: spawned {}", argv[0]);
        }
        Err(e) => {
            tracing::warn!(
                path = %dir.display(),
                error = %e,
                "show-in-folder: failed to spawn {}",
                argv[0],
            );
        }
    }
}

/// True when this process is running inside a Flatpak sandbox. Matches
/// the detection that `flowmux-cli` and the terminal pane already use.
fn in_flatpak_sandbox() -> bool {
    Path::new("/.flatpak-info").exists() || std::env::var_os("FLATPAK_ID").is_some()
}
