// SPDX-License-Identifier: GPL-3.0-or-later
//! Custom `gtk::Widget` subclass that renders the terminal pane through
//! GTK4's `gtk::Snapshot` / `GskRenderNode` API instead of cairo.
//!
//! The previous implementation was a `gtk::DrawingArea` with a
//! `set_draw_func` callback that issued cairo paint commands cell by
//! cell. Two structural problems showed up in that path:
//!
//! 1. Cairo paints in z-order, so any attempt to batch contiguous
//!    background fills had to be split into a separate pass from the
//!    text — a regression we hit twice when the bg run flushed after
//!    its text had already been drawn.
//! 2. Per-cell `set_source_rgba`, `rectangle`, `fill`, `move_to`,
//!    `show_layout` produced O(cols * rows) cairo state changes per
//!    frame. Heavy TUIs like opencode flooded the pipeline.
//!
//! Snapshot rendering replaces both with a node tree that GTK composites
//! on the GPU. The owner of the widget installs a closure via
//! [`TerminalSurface::set_snapshot_fn`]; the closure builds the tree
//! whenever GTK asks the widget to redraw.
//!
//! The widget also exposes [`TerminalSurface::connect_resize`] so the
//! pane runtime can react to size changes the same way it did to
//! `gtk::DrawingArea::connect_resize`. Internally the signal is fired
//! from `WidgetImpl::size_allocate` whenever the allocation changes.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Default)]
    pub struct TerminalSurface {
        pub(super) snapshot_fn:
            RefCell<Option<Box<dyn Fn(&gtk::Snapshot, &gtk::Widget)>>>,
        pub(super) resize_fn: RefCell<Option<Box<dyn Fn(i32, i32)>>>,
        pub(super) last_size: Cell<(i32, i32)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TerminalSurface {
        const NAME: &'static str = "FlowmuxTerminalSurface";
        type Type = super::TerminalSurface;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for TerminalSurface {}

    impl WidgetImpl for TerminalSurface {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            // Delegate to the closure installed by the pane runtime; if
            // none is set yet (during initial widget construction) GTK
            // simply gets an empty render, which paints the parent's
            // background through and is harmless.
            if let Some(f) = self.snapshot_fn.borrow().as_ref() {
                let widget: gtk::Widget = self.obj().clone().upcast();
                f(snapshot, &widget);
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            let prev = self.last_size.get();
            if prev != (width, height) {
                self.last_size.set((width, height));
                if let Some(f) = self.resize_fn.borrow().as_ref() {
                    f(width, height);
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct TerminalSurface(ObjectSubclass<imp::TerminalSurface>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for TerminalSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalSurface {
    pub fn new() -> Self {
        glib::Object::new::<Self>()
    }

    /// Install (or replace) the closure invoked from `snapshot()` to
    /// build the GskRenderNode tree for one frame.
    pub fn set_snapshot_fn<F>(&self, f: F)
    where
        F: Fn(&gtk::Snapshot, &gtk::Widget) + 'static,
    {
        *self.imp().snapshot_fn.borrow_mut() = Some(Box::new(f));
    }

    /// Install a resize handler that fires whenever GTK assigns a new
    /// allocation different from the previous one. Mirrors the
    /// `gtk::DrawingArea::resize` signal the pre-Snapshot widget used,
    /// so call sites that already wired up resize logic do not have to
    /// learn a new shape.
    pub fn connect_resize<F>(&self, f: F)
    where
        F: Fn(i32, i32) + 'static,
    {
        *self.imp().resize_fn.borrow_mut() = Some(Box::new(f));
    }
}
