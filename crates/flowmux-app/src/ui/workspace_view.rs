//! Render a workspace's pane tree as a recursive GTK widget tree.
//!
//! `Pane::Leaf` becomes a [`TerminalPane`] (or, later, a browser
//! widget); `Pane::Split { direction, ratio, first, second }` becomes
//! a `gtk::Paned` with the two children rendered recursively.
//!
//! State is owned by the controller; this module only builds widgets.

use crate::ui::browser_pane::BrowserPane;
use crate::ui::terminal_pane::{PaneCallbacks, TerminalPane};
use flowmux_core::{Pane, PaneContent, PaneId, SplitDirection, Surface, SurfaceKind};
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Default)]
pub struct PaneRegistry {
    pub terminals: HashMap<PaneId, TerminalPane>,
    pub browsers: HashMap<PaneId, BrowserPane>,
}

pub fn build_surface(
    surface: &Surface,
    callbacks: &PaneCallbacks,
    registry: Rc<RefCell<PaneRegistry>>,
) -> gtk::Widget {
    match &surface.kind {
        SurfaceKind::Terminal { cwd, shell } => {
            let argv = shell.clone().map(|s| vec![s]).unwrap_or_default();
            build_pane(&surface.root_pane, argv, cwd.clone(), callbacks, registry)
        }
        SurfaceKind::Browser { initial_url } => {
            // Walk to the first leaf and build a browser pane there.
            // For browser surfaces the tree is virtually always a single
            // leaf, but we keep the recursion so split-and-browse works.
            build_browser_subtree(
                &surface.root_pane,
                initial_url.as_deref(),
                registry,
            )
        }
    }
}

fn build_browser_subtree(
    pane: &Pane,
    initial_url: Option<&str>,
    registry: Rc<RefCell<PaneRegistry>>,
) -> gtk::Widget {
    match pane {
        Pane::Leaf { id, .. } => {
            let pane = BrowserPane::new(*id, initial_url);
            let widget = pane.root.clone().upcast::<gtk::Widget>();
            registry.borrow_mut().browsers.insert(*id, pane);
            widget
        }
        Pane::Split { direction, ratio, first, second, .. } => {
            let orient = match direction {
                SplitDirection::Horizontal => gtk::Orientation::Vertical,
                SplitDirection::Vertical => gtk::Orientation::Horizontal,
            };
            let paned = gtk::Paned::new(orient);
            paned.set_hexpand(true);
            paned.set_vexpand(true);
            paned.set_start_child(Some(&build_browser_subtree(first, initial_url, registry.clone())));
            paned.set_end_child(Some(&build_browser_subtree(second, None, registry)));
            let r = *ratio;
            paned.connect_realize(move |p| {
                let total = match p.orientation() {
                    gtk::Orientation::Horizontal => p.width(),
                    _ => p.height(),
                };
                if total > 0 {
                    p.set_position((total as f32 * r) as i32);
                }
            });
            paned.upcast()
        }
    }
}

fn build_pane(
    pane: &Pane,
    argv: Vec<String>,
    cwd: Option<std::path::PathBuf>,
    callbacks: &PaneCallbacks,
    registry: Rc<RefCell<PaneRegistry>>,
) -> gtk::Widget {
    match pane {
        Pane::Leaf { id, content } => match content {
            PaneContent::Terminal { .. } => {
                let pane = TerminalPane::spawn(*id, argv, cwd, callbacks.clone());
                let frame = gtk::Frame::new(None);
                frame.set_child(Some(&pane.widget));
                frame.add_css_class("flowmux-pane");
                registry.borrow_mut().terminals.insert(*id, pane);
                frame.upcast()
            }
            PaneContent::Browser { url } => {
                let pane = BrowserPane::new(*id, Some(url));
                let widget = pane.root.clone().upcast::<gtk::Widget>();
                registry.borrow_mut().browsers.insert(*id, pane);
                widget
            }
        },
        Pane::Split { direction, ratio, first, second, .. } => {
            let orient = match direction {
                SplitDirection::Horizontal => gtk::Orientation::Vertical,
                SplitDirection::Vertical => gtk::Orientation::Horizontal,
            };
            let paned = gtk::Paned::new(orient);
            paned.set_hexpand(true);
            paned.set_vexpand(true);
            let left = build_pane(first, argv.clone(), cwd.clone(), callbacks, registry.clone());
            let right = build_pane(second, argv, cwd, callbacks, registry);
            paned.set_start_child(Some(&left));
            paned.set_end_child(Some(&right));
            paned.set_resize_start_child(true);
            paned.set_resize_end_child(true);
            paned.set_shrink_start_child(false);
            paned.set_shrink_end_child(false);
            // Ratio is a hint; GTK measures children at allocation time
            // and we set `position` once the widget knows its size.
            let r = *ratio;
            paned.connect_realize(move |p| {
                let total = match p.orientation() {
                    gtk::Orientation::Horizontal => p.width(),
                    _ => p.height(),
                };
                if total > 0 {
                    p.set_position((total as f32 * r) as i32);
                }
            });
            paned.upcast()
        }
    }
}
