// SPDX-License-Identifier: GPL-3.0-or-later
//! Authoritative workspace state and its sidebar/pane widget projection.

use super::*;

#[derive(Clone)]
pub struct WorkspacePresenter {
    pub(super) store: StateStore,
    pub(super) sidebar: Sidebar,
    pub(super) stack: gtk::Stack,
    pub(super) surfaces: Rc<RefCell<HashMap<WorkspaceId, gtk::Widget>>>,
    pub(super) pane_registry: Rc<RefCell<PaneRegistry>>,
}

impl WorkspacePresenter {
    pub(super) fn new(
        store: StateStore,
        sidebar: Sidebar,
        stack: gtk::Stack,
        surfaces: Rc<RefCell<HashMap<WorkspaceId, gtk::Widget>>>,
        pane_registry: Rc<RefCell<PaneRegistry>>,
    ) -> Self {
        Self {
            store,
            sidebar,
            stack,
            surfaces,
            pane_registry,
        }
    }
}
