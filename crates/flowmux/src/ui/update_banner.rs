// SPDX-License-Identifier: GPL-3.0-or-later
//! Side-panel banner for self-update. Renders [`BannerState`] into an
//! `adw::Banner` pinned above the side panel footer, spawns the
//! periodic release check, and starts the install when the user asks.

use crate::update::{self, BannerState, Event};
use std::cell::RefCell;
use std::rc::Rc;

/// Widget text for a banner state: `(title, button_label, revealed)`.
/// `None` for the button label hides the button. Pure so the mapping
/// stays unit-testable without GTK.
fn banner_props(state: &BannerState) -> (String, Option<&'static str>, bool) {
    use crate::update::Stage;
    match state {
        BannerState::Hidden => (String::new(), None, false),
        BannerState::Available(v) => (format!("FlowMux {v} is available"), Some("Update"), true),
        BannerState::Running(Stage::Fetching, v) => {
            (format!("Updating to {v} — downloading…"), None, true)
        }
        BannerState::Running(Stage::Installing, v) => (
            format!("Updating to {v} — building & installing…"),
            None,
            true,
        ),
        BannerState::Done(v) => (
            format!("FlowMux {v} installed — takes effect on next launch"),
            Some("Dismiss"),
            true,
        ),
        BannerState::Failed(message, v) => (
            format!("Update to {v} failed: {message} (see ~/.cache/flowmux/update.log)"),
            Some("Retry"),
            true,
        ),
    }
}

#[derive(Clone)]
pub struct UpdateBanner {
    banner: adw::Banner,
    state: Rc<RefCell<BannerState>>,
    tx: async_channel::Sender<Event>,
    tokio_handle: Option<tokio::runtime::Handle>,
}

impl UpdateBanner {
    /// Build the (hidden) banner and start the release check on
    /// `tokio_handle`. Without a handle — tests, degraded startup —
    /// the banner stays permanently hidden.
    pub fn new(tokio_handle: Option<tokio::runtime::Handle>) -> Self {
        let banner = adw::Banner::new("");
        banner.set_revealed(false);

        let (tx, rx) = async_channel::unbounded::<Event>();
        let this = Self {
            banner: banner.clone(),
            state: Rc::new(RefCell::new(BannerState::Hidden)),
            tx,
            tokio_handle: tokio_handle.clone(),
        };

        if let Some(handle) = &tokio_handle {
            handle.spawn(update::install::check_loop(this.tx.clone()));
        }

        let for_events = this.clone();
        gtk::glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = rx.recv().await {
                for_events.dispatch(event);
            }
        });

        let for_click = this.clone();
        banner.connect_button_clicked(move |banner| {
            let actionable = for_click.state.borrow().actionable_version();
            match actionable {
                Some(version) => {
                    if let Some(handle) = &for_click.tokio_handle {
                        handle.spawn(update::install::run_install(version, for_click.tx.clone()));
                    }
                }
                // Done state: the button is "Dismiss". Keep the state —
                // BannerState::Done ignores re-announcements of the
                // installed release, so the banner stays dormant until
                // a strictly newer tag appears.
                None => banner.set_revealed(false),
            }
        });

        this
    }

    pub fn widget(&self) -> &adw::Banner {
        &self.banner
    }

    fn dispatch(&self, event: Event) {
        let next = self.state.borrow().clone().apply(event);
        let (title, button, revealed) = banner_props(&next);
        self.banner.set_title(&title);
        self.banner.set_button_label(button);
        self.banner.set_revealed(revealed);
        *self.state.borrow_mut() = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::{check::Version, Stage};

    const V: Version = Version(0, 8, 0);

    #[test]
    fn hidden_state_reveals_nothing() {
        assert_eq!(
            banner_props(&BannerState::Hidden),
            (String::new(), None, false)
        );
    }

    #[test]
    fn available_offers_the_update_button() {
        let (title, button, revealed) = banner_props(&BannerState::Available(V));
        assert!(
            title.contains("0.8.0"),
            "title should name the release: {title}"
        );
        assert_eq!(button, Some("Update"));
        assert!(revealed);
    }

    #[test]
    fn running_shows_the_stage_and_no_button() {
        for (stage, needle) in [
            (Stage::Fetching, "downloading"),
            (Stage::Installing, "installing"),
        ] {
            let (title, button, revealed) = banner_props(&BannerState::Running(stage, V));
            assert!(title.contains(needle), "{title} should mention {needle}");
            assert_eq!(button, None, "no button while running");
            assert!(revealed);
        }
    }

    #[test]
    fn done_announces_next_launch_and_dismisses() {
        let (title, button, revealed) = banner_props(&BannerState::Done(V));
        assert!(title.contains("next launch"), "{title}");
        assert_eq!(button, Some("Dismiss"));
        assert!(revealed);
    }

    #[test]
    fn failed_offers_retry_and_points_at_the_log() {
        let (title, button, revealed) =
            banner_props(&BannerState::Failed("git fetch exited with 128".into(), V));
        assert!(title.contains("git fetch exited with 128"), "{title}");
        assert!(
            title.contains("update.log"),
            "{title} should point at the log"
        );
        assert_eq!(button, Some("Retry"));
        assert!(revealed);
    }
}
