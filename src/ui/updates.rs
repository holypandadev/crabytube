//! Updates in the interface: the automatic check, what Settings and Home say about a newer
//! version, and the download and install of one. The work itself is in `crate::updater`.

use super::*;
use crate::updater::{Available, UpdateError};

/// The first automatic check waits this long after the start, so it never competes with Home.
pub(super) const FIRST_CHECK_DELAY: std::time::Duration = std::time::Duration::from_secs(20);
/// Between automatic checks while the app runs.
pub(super) const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Where an update stands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum UpdateState {
    /// Nothing has been asked yet in this session.
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Available),
    Downloading {
        available: Available,
        received: u64,
        total: u64,
    },
    /// The installer has started; the app quits.
    Installing,
    /// What went wrong, in English (the key of its translation), and the update it concerned.
    Failed {
        reason: &'static str,
        available: Option<Available>,
    },
}

impl UpdateState {
    /// The newer version this state is about, if any.
    pub(super) fn available(&self) -> Option<&Available> {
        match self {
            UpdateState::Available(available)
            | UpdateState::Downloading { available, .. }
            | UpdateState::Failed {
                available: Some(available),
                ..
            } => Some(available),
            _ => None,
        }
    }

    /// Whether a check, a download or an install is under way.
    fn busy(&self) -> bool {
        matches!(
            self,
            UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Installing
        )
    }
}

/// The sentence Settings shows for a failure.
fn reason_for(error: &UpdateError) -> &'static str {
    match error {
        UpdateError::Network(_) => "Could not reach GitHub. Try again later.",
        UpdateError::Signature => {
            "The update is not signed with CrabyTube's release key, so it was not used."
        }
        UpdateError::Manifest(_) => "The update information could not be read.",
        UpdateError::Mismatch => "The download did not match the release and was deleted.",
        UpdateError::Storage(_) => "The update could not be saved.",
        UpdateError::Install(_) => "The update could not be installed.",
    }
}

impl App {
    /// The update messages of `update_inner`, which routes them here.
    pub(super) fn update_updates(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::CheckForUpdates { manual } => {
                if !self.update_install.checks()
                    || (!manual && !self.preferences.check_updates)
                    || self.update.busy()
                {
                    return Task::none();
                }
                if manual {
                    self.update = UpdateState::Checking;
                } else if self.update.available().is_some() {
                    // An automatic check leaves an update already found as it is.
                    return Task::none();
                }
                // An automatic check runs out of sight: Settings changes only when it finds a
                // version, so it never replaces what a check by hand has just said.
                let install = self.update_install.clone();
                Task::perform(
                    async move {
                        crate::updater::check(&crate::updater::Feed::current(), &install).await
                    },
                    move |result| Message::UpdateChecked { manual, result },
                )
            }
            Message::UpdateChecked { manual, result } => {
                // A late answer does not interrupt a download or an install under way.
                if matches!(
                    self.update,
                    UpdateState::Downloading { .. } | UpdateState::Installing
                ) {
                    return Task::none();
                }
                match result {
                    Ok(Some(available)) => {
                        tracing::info!(version = %available.version, "a newer version is available");
                        self.update = UpdateState::Available(available);
                    }
                    Ok(None) => self.update = UpdateState::UpToDate,
                    Err(error) => {
                        tracing::info!(%error, manual, "the update check failed");
                        // An automatic check that fails changes nothing on screen; the next one
                        // tries again.
                        if manual {
                            self.update = UpdateState::Failed {
                                reason: reason_for(&error),
                                available: None,
                            };
                        }
                    }
                }
                Task::none()
            }
            Message::InstallUpdate => {
                let Some(available) = self.update.available().cloned() else {
                    return Task::none();
                };
                if self.update.busy() {
                    return Task::none();
                }
                let Some(package) = available
                    .package
                    .clone()
                    .filter(|_| self.update_install.installs())
                else {
                    crate::updater::open_page(&available.notes_url);
                    return Task::none();
                };
                self.update = UpdateState::Downloading {
                    available,
                    received: 0,
                    total: package.size,
                };
                let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
                let progress_stream =
                    futures_util::stream::unfold(progress_rx, |mut receiver| async {
                        receiver.recv().await.map(|step| (step, receiver))
                    });
                let progress = Task::run(progress_stream, |(received, total)| {
                    Message::UpdateProgress { received, total }
                });
                // One message per whole percent, not one per piece of the download.
                let last = std::sync::atomic::AtomicU64::new(u64::MAX);
                let download = Task::perform(
                    crate::updater::download(package, move |received, total| {
                        let percent = received.saturating_mul(100) / total.max(1);
                        if last.swap(percent, std::sync::atomic::Ordering::Relaxed) != percent {
                            let _ = progress_tx.send((received, total));
                        }
                    }),
                    Message::UpdateDownloaded,
                );
                Task::batch([progress, download])
            }
            Message::UpdateProgress { received, total } => {
                if let UpdateState::Downloading {
                    received: shown,
                    total: of,
                    ..
                } = &mut self.update
                {
                    *shown = received;
                    *of = total;
                }
                Task::none()
            }
            Message::UpdateDownloaded(result) => {
                let UpdateState::Downloading { available, .. } = self.update.clone() else {
                    return Task::none();
                };
                match result.and_then(|path| crate::updater::install(&path, &self.update_install)) {
                    Ok(()) => {
                        tracing::info!(version = %available.version, "installing the update; quitting");
                        self.update = UpdateState::Installing;
                        self.exit_task()
                    }
                    Err(error) => {
                        tracing::warn!(%error, "the update was not installed");
                        self.update = UpdateState::Failed {
                            reason: reason_for(&error),
                            available: Some(available),
                        };
                        Task::none()
                    }
                }
            }
            Message::OpenUpdatePage => {
                if let Some(available) = self.update.available() {
                    crate::updater::open_page(&available.notes_url);
                }
                Task::none()
            }
            _ => unreachable!("update_inner routes only this group here"),
        }
    }

    /// The start of the first automatic check, a while after the app opens.
    pub(super) fn first_update_check_task(&self) -> Task<Message> {
        if !self.update_install.checks() {
            return Task::none();
        }
        Task::perform(tokio::time::sleep(FIRST_CHECK_DELAY), |()| {
            Message::CheckForUpdates { manual: false }
        })
    }

    /// The daily check while the app runs, when checks are on.
    pub(super) fn update_check_subscription(&self) -> Subscription<Message> {
        if self.update_install.checks() && self.preferences.check_updates {
            iced::time::every(CHECK_EVERY).map(|_| Message::CheckForUpdates { manual: false })
        } else {
            Subscription::none()
        }
    }

    /// What Settings says about updates, and the button beside it.
    pub(super) fn update_status(&self) -> (String, Option<(&'static str, Message)>) {
        let running = env!("CARGO_PKG_VERSION");
        let check = Some((
            self.tr_static("Check now"),
            Message::CheckForUpdates { manual: true },
        ));
        match &self.update {
            UpdateState::Idle => (
                self.tr_static("CrabyTube {} is installed.")
                    .replacen("{}", running, 1),
                check,
            ),
            UpdateState::Checking => (self.tr("Checking for updates…").into_owned(), None),
            UpdateState::UpToDate => (
                self.tr_static("CrabyTube {} is up to date.")
                    .replacen("{}", running, 1),
                check,
            ),
            UpdateState::Available(available) => (
                self.tr_static("Version {} is available.")
                    .replacen("{}", &available.version, 1),
                Some(self.update_action(available)),
            ),
            UpdateState::Downloading {
                available,
                received,
                total,
            } => (
                self.tr_static("Downloading version {}… {}%")
                    .replacen("{}", &available.version, 1)
                    .replacen(
                        "{}",
                        &(received.saturating_mul(100) / (*total).max(1)).to_string(),
                        1,
                    ),
                None,
            ),
            UpdateState::Installing => (self.tr("Installing the update…").into_owned(), None),
            UpdateState::Failed { reason, available } => (
                self.tr(reason).into_owned(),
                match available {
                    Some(available) => Some(self.update_action(available)),
                    None => check,
                },
            ),
        }
    }

    /// Update now where the app installs it, the release page elsewhere.
    fn update_action(&self, available: &Available) -> (&'static str, Message) {
        if available.package.is_some() && self.update_install.installs() {
            (self.tr_static("Update now"), Message::InstallUpdate)
        } else {
            (self.tr_static("Download page"), Message::OpenUpdatePage)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::{Install, Package};

    fn available(with_package: bool) -> Available {
        Available {
            version: "9.9.9".into(),
            notes_url: "https://github.com/holypandadev/crabytube/releases/tag/v9.9.9".into(),
            package: with_package.then(|| Package {
                kind: "windows-msi".into(),
                url: "https://example.invalid/p.msi".into(),
                size: 10,
                sha256: "a".repeat(64),
            }),
        }
    }

    #[test]
    fn every_failure_has_a_translation_in_every_language() {
        let errors = [
            UpdateError::Network(String::new()),
            UpdateError::Signature,
            UpdateError::Manifest(String::new()),
            UpdateError::Mismatch,
            UpdateError::Storage(String::new()),
            UpdateError::Install(String::new()),
        ];
        for language in crate::preferences::UiLanguage::ALL {
            if language == crate::preferences::UiLanguage::English {
                continue;
            }
            for error in &errors {
                let reason = reason_for(error);
                assert_ne!(
                    crate::i18n::translate(language, reason),
                    reason,
                    "{language:?}: {reason}"
                );
            }
        }
    }

    #[test]
    fn a_found_update_is_offered_and_a_failed_automatic_check_says_nothing() {
        let mut app = App {
            update_install: Install::WindowsMsi,
            ..App::default()
        };
        drop(app.update(Message::UpdateChecked {
            manual: false,
            result: Err(UpdateError::Network("offline".into())),
        }));
        assert_eq!(app.update, UpdateState::Idle);
        drop(app.update(Message::UpdateChecked {
            manual: true,
            result: Err(UpdateError::Signature),
        }));
        assert!(matches!(
            app.update,
            UpdateState::Failed {
                available: None,
                ..
            }
        ));
        drop(app.update(Message::UpdateChecked {
            manual: false,
            result: Ok(Some(available(true))),
        }));
        let (status, action) = app.update_status();
        assert_eq!(status, "Version 9.9.9 is available.");
        assert!(matches!(
            action,
            Some(("Update now", Message::InstallUpdate))
        ));
        // An automatic check leaves the update found on screen.
        drop(app.update(Message::CheckForUpdates { manual: false }));
        assert_eq!(app.update, UpdateState::Available(available(true)));
    }

    #[test]
    fn an_automatic_check_never_replaces_what_a_check_by_hand_said() {
        let mut app = App {
            update_install: Install::WindowsMsi,
            ..App::default()
        };
        // Check now fails, shortly after the start.
        assert!(
            app.update(Message::CheckForUpdates { manual: true })
                .units()
                > 0
        );
        drop(app.update(Message::UpdateChecked {
            manual: true,
            result: Err(UpdateError::Network("404".into())),
        }));
        let failed = app.update.clone();
        assert!(matches!(failed, UpdateState::Failed { .. }));
        // The check of the start runs after it, out of sight, and fails too.
        assert!(
            app.update(Message::CheckForUpdates { manual: false })
                .units()
                > 0
        );
        assert_eq!(app.update, failed);
        drop(app.update(Message::UpdateChecked {
            manual: false,
            result: Err(UpdateError::Network("404".into())),
        }));
        assert_eq!(app.update, failed);
        // When it finds a version, that is shown.
        drop(app.update(Message::UpdateChecked {
            manual: false,
            result: Ok(Some(available(true))),
        }));
        assert_eq!(app.update, UpdateState::Available(available(true)));
    }

    #[test]
    fn a_late_answer_does_not_interrupt_a_download() {
        let mut app = App {
            update_install: Install::WindowsMsi,
            ..App::default()
        };
        app.update = UpdateState::Downloading {
            available: available(true),
            received: 5,
            total: 10,
        };
        drop(app.update(Message::UpdateChecked {
            manual: false,
            result: Ok(None),
        }));
        assert!(matches!(app.update, UpdateState::Downloading { .. }));
    }

    #[test]
    fn a_copy_that_cannot_install_offers_the_release_page() {
        let mut app = App {
            update_install: Install::Manual,
            ..App::default()
        };
        app.update = UpdateState::Available(available(false));
        assert!(matches!(
            app.update_status().1,
            Some(("Download page", Message::OpenUpdatePage))
        ));
        // Installing from such a copy only opens the page; nothing is downloaded.
        drop(app.update(Message::InstallUpdate));
        assert_eq!(app.update, UpdateState::Available(available(false)));
    }

    #[test]
    fn nothing_is_checked_where_a_package_manager_updates_the_app() {
        let mut app = App {
            update_install: Install::Managed,
            ..App::default()
        };
        let check = app.update(Message::CheckForUpdates { manual: true });
        assert_eq!(check.units(), 0);
        assert_eq!(app.update, UpdateState::Idle);
        assert_eq!(app.first_update_check_task().units(), 0);
    }

    #[test]
    fn automatic_checks_follow_the_setting_and_a_manual_one_always_runs() {
        let mut app = App {
            update_install: Install::WindowsMsi,
            ..App::default()
        };
        app.preferences.check_updates = false;
        assert_eq!(
            app.update(Message::CheckForUpdates { manual: false })
                .units(),
            0
        );
        assert!(
            app.update(Message::CheckForUpdates { manual: true })
                .units()
                > 0
        );
        assert_eq!(app.update, UpdateState::Checking);
        // A second check does not start over one under way.
        assert_eq!(
            app.update(Message::CheckForUpdates { manual: true })
                .units(),
            0
        );
    }

    #[test]
    fn the_download_shows_its_progress_in_whole_percents() {
        let mut app = App {
            update_install: Install::WindowsMsi,
            ..App::default()
        };
        app.update = UpdateState::Downloading {
            available: available(true),
            received: 0,
            total: 200,
        };
        drop(app.update(Message::UpdateProgress {
            received: 50,
            total: 200,
        }));
        assert_eq!(app.update_status().0, "Downloading version 9.9.9… 25%");
        // A download that does not match leaves the update offered again, with the reason.
        drop(app.update(Message::UpdateDownloaded(Err(UpdateError::Mismatch))));
        let (status, action) = app.update_status();
        assert_eq!(
            status,
            "The download did not match the release and was deleted."
        );
        assert!(matches!(
            action,
            Some(("Update now", Message::InstallUpdate))
        ));
    }
}
