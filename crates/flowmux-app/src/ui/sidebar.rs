//! Workspace sidebar (cmux's vertical-tabs left panel).
//!
//! Each row shows: workspace name, current branch, linked PR badge
//! (if any), listening ports, and the latest unread notification body.
//! Selection emits a callback so the main window can swap the visible
//! surface stack.

use flowmux_core::{PrState, Workspace, WorkspaceId};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
pub struct Sidebar {
    pub root: gtk::ScrolledWindow,
    pub list: gtk::ListBox,
    rows: Rc<RefCell<Vec<(WorkspaceId, gtk::ListBoxRow)>>>,
}

impl Sidebar {
    pub fn new<F: Fn(WorkspaceId) + 'static>(on_select: F) -> Self {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("navigation-sidebar");

        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&list));

        let rows: Rc<RefCell<Vec<(WorkspaceId, gtk::ListBoxRow)>>> =
            Rc::new(RefCell::new(Vec::new()));

        let rows_for_cb = rows.clone();
        list.connect_row_selected(move |_, selected| {
            if let Some(row) = selected {
                if let Some((id, _)) = rows_for_cb
                    .borrow()
                    .iter()
                    .find(|(_, r)| r == row)
                    .cloned()
                {
                    on_select(id);
                }
            }
        });

        Self { root: scroll, list, rows }
    }

    pub fn upsert(&self, ws: &Workspace) {
        let mut rows = self.rows.borrow_mut();
        if let Some((_, row)) = rows.iter().find(|(id, _)| *id == ws.id).cloned() {
            row.set_child(Some(&row_widget(ws)));
            return;
        }
        let row = gtk::ListBoxRow::new();
        row.set_child(Some(&row_widget(ws)));
        self.list.append(&row);
        rows.push((ws.id, row));
    }

    pub fn remove(&self, id: WorkspaceId) {
        let mut rows = self.rows.borrow_mut();
        if let Some(idx) = rows.iter().position(|(wid, _)| *wid == id) {
            self.list.remove(&rows[idx].1);
            rows.swap_remove(idx);
        }
    }
}

fn row_widget(ws: &Workspace) -> gtk::Widget {
    let v = gtk::Box::new(gtk::Orientation::Vertical, 2);
    v.set_margin_top(8);
    v.set_margin_bottom(8);
    v.set_margin_start(10);
    v.set_margin_end(10);

    let title = gtk::Label::new(Some(&ws.name));
    title.set_halign(gtk::Align::Start);
    title.add_css_class("heading");
    v.append(&title);

    if let Some(git) = &ws.git {
        let h = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let branch = gtk::Label::new(Some(&format!("⎇ {}", git.branch)));
        branch.set_halign(gtk::Align::Start);
        branch.add_css_class("dim-label");
        branch.add_css_class("caption");
        h.append(&branch);
        if let Some(pr) = &git.linked_pr {
            let badge = gtk::Label::new(Some(&format!("#{}", pr.number)));
            badge.add_css_class("caption");
            badge.add_css_class(match pr.state {
                PrState::Open => "success",
                PrState::Merged => "accent",
                PrState::Closed => "warning",
                PrState::Draft => "dim-label",
            });
            h.append(&badge);
        }
        v.append(&h);
    }

    if !ws.listening_ports.is_empty() {
        let ports = ws
            .listening_ports
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let p = gtk::Label::new(Some(&format!(":: {ports}")));
        p.set_halign(gtk::Align::Start);
        p.add_css_class("caption");
        p.add_css_class("dim-label");
        v.append(&p);
    }

    v.upcast()
}
