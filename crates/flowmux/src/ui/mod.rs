// SPDX-License-Identifier: GPL-3.0-or-later
pub mod browser_pane;
pub mod options_dialog;
pub mod popover_pos;
pub mod sidebar;
pub mod terminal_pane;
pub mod terminal_surface;
pub mod window;
pub mod workspace_view;

pub use window::{spawn_dispatch_loop, WindowController};
