// SPDX-License-Identifier: GPL-3.0-or-later
//! Self-update: detect a newer release tag on the flowmux repository,
//! and on request bring a managed clone to that tag and run the
//! platform install script so the next launch runs the new version.
//!
//! Split: [`check`] is the pure, unit-tested core (version parsing,
//! command plan); [`install`] executes that plan on the tokio runtime;
//! `ui::update_banner` renders the state in the side panel.

pub mod check;
pub mod install;

use check::Version;
use std::sync::Mutex;

/// Newest release seen by the background check, mirrored here so the
/// About popup can render an "update available" line without plumbing
/// a channel into the options dialog.
pub static AVAILABLE: Mutex<Option<Version>> = Mutex::new(None);

/// Progress reported by the background install task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Bringing the managed clone to the release tag.
    Fetching,
    /// Running the platform install script (build + install).
    Installing,
}

/// Events flowing from the tokio side to the side-panel banner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A newer release exists.
    Available(Version),
    /// Install progress.
    Stage(Stage),
    /// Install finished; takes effect on next launch.
    Done(Version),
    /// Install failed; message is a short summary for the banner.
    Failed(String),
}

/// Side-panel banner state. Pure so transitions stay unit-testable;
/// the GTK adapter only maps a state to widget properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BannerState {
    Hidden,
    /// Update to `Version` can be started.
    Available(Version),
    /// Install running; keep the target version for retry/labels.
    Running(Stage, Version),
    Done(Version),
    /// Install failed; retry targets `Version`.
    Failed(String, Version),
}

impl BannerState {
    /// Fold an [`Event`] into the current state.
    pub fn apply(self, event: Event) -> BannerState {
        match (self, event) {
            // A running install owns the banner; periodic re-checks wait.
            (state @ BannerState::Running(..), Event::Available(_)) => state,
            // The installed release being re-announced is not news.
            (BannerState::Done(installed), Event::Available(v)) if v <= installed => {
                BannerState::Done(installed)
            }
            (_, Event::Available(v)) => BannerState::Available(v),
            (state, Event::Stage(stage)) => match state {
                BannerState::Available(v)
                | BannerState::Running(_, v)
                | BannerState::Failed(_, v)
                | BannerState::Done(v) => BannerState::Running(stage, v),
                BannerState::Hidden => BannerState::Hidden,
            },
            (_, Event::Done(v)) => BannerState::Done(v),
            (state, Event::Failed(message)) => match state {
                BannerState::Available(v)
                | BannerState::Running(_, v)
                | BannerState::Failed(_, v)
                | BannerState::Done(v) => BannerState::Failed(message, v),
                BannerState::Hidden => BannerState::Hidden,
            },
        }
    }

    /// True when clicking the banner button should start an install
    /// (initial attempt or retry) targeting the returned version.
    pub fn actionable_version(&self) -> Option<Version> {
        match self {
            BannerState::Available(v) | BannerState::Failed(_, v) => Some(*v),
            BannerState::Hidden | BannerState::Running(..) | BannerState::Done(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: Version = Version(0, 7, 1);
    const V2: Version = Version(0, 8, 0);

    #[test]
    fn available_shows_the_offer() {
        assert_eq!(
            BannerState::Hidden.apply(Event::Available(V1)),
            BannerState::Available(V1)
        );
    }

    #[test]
    fn a_newer_release_during_the_offer_updates_the_offer() {
        assert_eq!(
            BannerState::Available(V1).apply(Event::Available(V2)),
            BannerState::Available(V2)
        );
    }

    #[test]
    fn periodic_check_does_not_disturb_a_running_install() {
        let running = BannerState::Running(Stage::Installing, V1);
        assert_eq!(running.clone().apply(Event::Available(V2)), running);
    }

    #[test]
    fn stages_progress_while_running() {
        assert_eq!(
            BannerState::Available(V1).apply(Event::Stage(Stage::Fetching)),
            BannerState::Running(Stage::Fetching, V1)
        );
        assert_eq!(
            BannerState::Running(Stage::Fetching, V1).apply(Event::Stage(Stage::Installing)),
            BannerState::Running(Stage::Installing, V1)
        );
    }

    #[test]
    fn done_and_failed_terminate_a_run() {
        assert_eq!(
            BannerState::Running(Stage::Installing, V1).apply(Event::Done(V1)),
            BannerState::Done(V1)
        );
        assert_eq!(
            BannerState::Running(Stage::Fetching, V1).apply(Event::Failed("boom".into())),
            BannerState::Failed("boom".into(), V1)
        );
    }

    #[test]
    fn done_ignores_re_announcement_of_the_installed_release() {
        assert_eq!(
            BannerState::Done(V1).apply(Event::Available(V1)),
            BannerState::Done(V1)
        );
        // …but a strictly newer one re-opens the offer.
        assert_eq!(
            BannerState::Done(V1).apply(Event::Available(V2)),
            BannerState::Available(V2)
        );
    }

    #[test]
    fn only_available_and_failed_are_actionable() {
        assert_eq!(BannerState::Available(V1).actionable_version(), Some(V1));
        assert_eq!(
            BannerState::Failed("e".into(), V1).actionable_version(),
            Some(V1)
        );
        assert_eq!(BannerState::Hidden.actionable_version(), None);
        assert_eq!(
            BannerState::Running(Stage::Fetching, V1).actionable_version(),
            None
        );
        assert_eq!(BannerState::Done(V1).actionable_version(), None);
    }
}
