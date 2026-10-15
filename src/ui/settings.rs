//! Settings: outlined boxes in two columns, each box going to the shorter column.
//! Only controls that change real behaviour are listed; every change saves immediately.

use super::messages::Confirmation;
use super::shell::fit;
use super::theme::{
    self, ALERT, DIM, FADED, INK, MONO, RULE, TEXT_BODY, TEXT_CAPTION, TEXT_DISPLAY, TEXT_SECTION,
    hrule, icon, label, toggle,
};
use super::{App, Message, Preference};
use crate::playback::PlaybackState;
use crate::preferences::{UiDensity, UiLanguage};
use iced::widget::{
    Column, Space, button, column, container, pick_list, row, scrollable, slider, text,
};
use iced::{Alignment, Element, Length, Size};

const BOX_PADDING: f32 = 20.0;
const GRID_GAP: f32 = 24.0;
const TWO_COLUMNS_FROM: f32 = 880.0;
/// Below this width each setting puts its control under its label, so the control can take the
/// full width of the box instead of a share of it.
const STACK_ROWS_BELOW: f32 = 520.0;
/// Estimated box chrome (padding, title and rules) and height per row. They only steer
/// the column choice; nothing is sized from them.
const BOX_CHROME_ESTIMATE: f32 = 80.0;
const ROW_ESTIMATE: f32 = 64.0;

/// A settings box with its row count, which estimates its height for the column layout.
struct Section<'a> {
    rows: usize,
    element: Element<'a, Message>,
}

impl Section<'_> {
    fn estimated_height(&self) -> f32 {
        BOX_CHROME_ESTIMATE + self.rows as f32 * ROW_ESTIMATE
    }
}

/// The column (0 = left, 1 = right) for each height, in order: every box goes to the
/// shorter column so far, and a tie goes left.
fn masonry_columns(heights: &[f32]) -> Vec<usize> {
    let mut totals = [0.0_f32; 2];
    heights
        .iter()
        .map(|height| {
            let column = usize::from(totals[1] < totals[0]);
            totals[column] += height;
            column
        })
        .collect()
}

/// A size in MiB with one decimal place, with the decimal separator of the language.
fn mib(bytes: u64, language: UiLanguage) -> String {
    format!(
        "{} MiB",
        crate::i18n::decimal(language, bytes as f64 / 1_048_576.0, 1)
    )
}

impl App {
    /// `top` is the height of the floating tabs above the page; the content scrolls under them.
    pub(super) fn settings_page(&self, area: Size) -> Element<'_, Message> {
        let width = area.width.max(300.0);
        let two_columns = width >= TWO_COLUMNS_FROM;
        let box_width = if two_columns {
            ((width - GRID_GAP) / 2.0).floor()
        } else {
            width
        };
        let narrow = width < 700.0;
        let title = column![
            // A narrow window takes the smaller title, as Favorites does.
            text(self.tr("Settings"))
                .size(if narrow {
                    theme::TEXT_TITLE
                } else {
                    TEXT_DISPLAY
                })
                .font(MONO),
            label(self.tr("Customize your experience").into_owned(), TEXT_BODY),
        ]
        .spacing(6)
        .width(Length::Fill);
        let reset = button(
            row![
                icon("repeat", 16.0, INK),
                label(self.tr("Reset").into_owned(), TEXT_BODY)
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press(Message::AskResetSettings)
        .padding([12, 24])
        .style(theme::outline_button);
        // On a narrow page the button goes under the title: beside it, a long word such as
        // "Einstellungen" ran under "Zurücksetzen".
        let header: Element<'_, Message> = if narrow {
            column![title, reset].spacing(16).into()
        } else {
            row![
                title,
                if width >= 760.0 {
                    Element::from(
                        label(
                            self.tr("Changes save automatically").into_owned(),
                            TEXT_CAPTION,
                        )
                        .color(FADED),
                    )
                } else {
                    Space::new().width(0).into()
                },
                reset,
            ]
            .spacing(24)
            .align_y(Alignment::Center)
            .into()
        };

        let sections = [
            self.appearance_box(box_width),
            self.playback_box(box_width),
            self.audio_box(box_width),
            self.general_box(box_width),
            self.lyrics_box(box_width),
            self.integrations_box(box_width),
            self.storage_box(box_width),
            self.about_box(box_width),
        ];
        let grid: Element<'_, Message> = if two_columns {
            let heights: Vec<f32> = sections
                .iter()
                .map(|section| section.estimated_height())
                .collect();
            let mut columns: [Vec<Element<'_, Message>>; 2] = [Vec::new(), Vec::new()];
            for (section, index) in sections.into_iter().zip(masonry_columns(&heights)) {
                columns[index].push(section.element);
            }
            let [left, right] = columns;
            row![
                column(left).spacing(GRID_GAP).width(box_width),
                column(right).spacing(GRID_GAP).width(box_width),
            ]
            .spacing(GRID_GAP)
            .align_y(Alignment::Start)
            .into()
        } else {
            column(sections.into_iter().map(|section| section.element))
                .spacing(GRID_GAP)
                .into()
        };
        let reset_confirmation: Element<'_, Message> =
            match self.confirmation.as_ref().map(|pending| &pending.kind) {
                Some(Confirmation::Reset) => self.reset_confirmation(),
                _ => Space::new().height(0).into(),
            };
        scrollable(
            column![
                Space::new().height(8.0),
                header,
                reset_confirmation,
                Space::new().height(28),
                grid,
                Space::new().height(32),
            ]
            .width(Length::Fill),
        )
        .id("settings-content")
        .direction(theme::vbar())
        .style(theme::scroll_style)
        .height(Length::Fill)
        .into()
    }

    fn settings_box<'a>(
        &'a self,
        title: &'static str,
        width: f32,
        rows: Vec<Element<'a, Message>>,
    ) -> Section<'a> {
        let mut body = column![
            text(self.tr(title)).size(TEXT_SECTION).font(MONO),
            Space::new().height(14),
            hrule(INK),
        ];
        let count = rows.len();
        for (index, line) in rows.into_iter().enumerate() {
            body = body.push(line);
            if index + 1 < count {
                body = body.push(hrule(RULE));
            }
        }
        Section {
            rows: count,
            element: container(body)
                .width(width)
                .padding(BOX_PADDING)
                .style(theme::boxed)
                .into(),
        }
    }

    /// Cancel and confirm for an inline confirmation. Cancel leaves the setting as it was.
    fn confirm_buttons(&self, confirm: &'static str, message: Message) -> Element<'_, Message> {
        row![
            button(label(self.tr("Cancel").into_owned(), TEXT_CAPTION))
                .on_press(Message::CancelConfirmation)
                .padding([6, 12])
                .style(theme::outline_button),
            button(label(self.tr(confirm).into_owned(), TEXT_CAPTION))
                .on_press(message)
                .padding([6, 12])
                .style(theme::outline_button),
        ]
        .spacing(8)
        .into()
    }

    /// The Reset confirmation, shown under the header while it waits for its second press.
    fn reset_confirmation(&self) -> Element<'_, Message> {
        column![
            label(self.tr("Reset settings?").into_owned(), TEXT_BODY),
            text(self.tr("Language, window size, volume and lyrics offsets stay."))
                .size(TEXT_CAPTION)
                .font(MONO)
                .color(DIM),
            row![
                Space::new().width(Length::Fill),
                self.confirm_buttons("Confirm", Message::ResetSettings),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(8)
        .padding([14, 0])
        .into()
    }

    /// What an import brings, counted from the backup that was read.
    fn import_summary(&self, snapshot: &crate::preferences::Snapshot) -> String {
        let filled = self
            .tr_static("Brings {} favorites and {} queued songs, plus its settings.")
            .replacen("{}", &snapshot.favorites.len().to_string(), 1)
            .replacen("{}", &snapshot.queue.len().to_string(), 1);
        // In English the counts then take the form that agrees with them.
        self.tr_string(filled)
    }

    /// The search history row. Clearing it, or turning it off, also clears "Jump back in" when that
    /// shelf has anything, so the row says so and asks first.
    fn history_setting(&self, width: f32) -> Element<'_, Message> {
        let searches = self.search_history.len();
        let shelf = !self.recent.is_empty();
        match self.confirmation.as_ref().map(|pending| &pending.kind) {
            Some(Confirmation::ClearHistory) => self.setting(
                "Search history",
                Some(self.tr_static("Clear history and Jump back in?").into()),
                self.confirm_buttons("Confirm", Message::ClearHistory),
                width,
            ),
            Some(Confirmation::TurnOffHistory) => self.setting(
                "Search history",
                Some(
                    self.tr_static("Turn off history and remove Jump back in?")
                        .into(),
                ),
                self.confirm_buttons("Confirm", Message::ConfirmTurnOffHistory),
                width,
            ),
            _ => {
                let hint = if shelf {
                    self.tr_static(
                        "Saved searches: {}. Clearing or turning this off also removes 'Jump back in'.",
                    )
                } else {
                    self.tr_static("Saved searches: {}.")
                }
                .replacen("{}", &searches.to_string(), 1);
                let remember = self.preferences.remember_history;
                self.setting(
                    "Search history",
                    Some(hint),
                    row![
                        button(label(self.tr("Clear").into_owned(), TEXT_CAPTION))
                            .on_press_maybe(
                                (searches > 0 || shelf).then_some(Message::AskClearHistory)
                            )
                            .padding([5, 10])
                            .style(theme::outline_button),
                        toggle(
                            remember,
                            Message::Preference(Preference::History(!remember))
                        ),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .into(),
                    width,
                )
            }
        }
    }

    /// One setting: label (and optional hint) on the left, control on the right. Narrow windows
    /// put the control under the label.
    fn setting<'a>(
        &self,
        title: &'static str,
        hint: Option<String>,
        control: Element<'a, Message>,
        _width: f32,
    ) -> Element<'a, Message> {
        let left = self.setting_label(title, hint);
        if self.area.width < STACK_ROWS_BELOW {
            return column![left, control].spacing(10).padding([14, 0]).into();
        }
        row![container(left).width(Length::Fill), control]
            .spacing(16)
            .padding([14, 0])
            .align_y(Alignment::Center)
            .into()
    }

    /// One setting with its control under the label at any width, for a control that needs room.
    fn setting_above<'a>(
        &self,
        title: &'static str,
        hint: Option<String>,
        control: Element<'a, Message>,
    ) -> Element<'a, Message> {
        column![self.setting_label(title, hint), control]
            .spacing(10)
            .padding([14, 0])
            .into()
    }

    /// The label of a setting, with its hint under it.
    fn setting_label<'a>(&self, title: &'static str, hint: Option<String>) -> Column<'a, Message> {
        let mut left = column![label(self.tr(title).into_owned(), TEXT_BODY)].spacing(4);
        if let Some(hint) = hint {
            left = left.push(
                text(self.tr_string(hint))
                    .size(TEXT_CAPTION)
                    .font(MONO)
                    .color(DIM)
                    .width(Length::Fill),
            );
        }
        left
    }

    /// Drop-downs shrink on narrow windows so rows never overflow.
    fn picker_width(&self) -> f32 {
        let two_columns = (self.area.width) >= TWO_COLUMNS_FROM;
        let box_width = if two_columns {
            (self.area.width - GRID_GAP) / 2.0
        } else {
            self.area.width
        };
        (box_width * 0.4).clamp(120.0, 220.0).floor()
    }

    fn picker<'a, T>(
        &self,
        options: Vec<T>,
        selected: Option<T>,
        on_select: impl Fn(T) -> Message + 'a,
    ) -> Element<'a, Message>
    where
        T: ToString + PartialEq + Clone + 'a,
    {
        pick_list(options, selected, on_select)
            .font(MONO)
            .text_size(TEXT_BODY)
            .padding([8, 12])
            .width(self.picker_width())
            .style(theme::pick_style)
            .menu_style(theme::menu_style)
            .into()
    }

    fn appearance_box(&self, width: f32) -> Section<'_> {
        let p = &self.preferences;
        self.settings_box(
            "Appearance",
            width,
            vec![
                self.setting(
                    "Language",
                    None,
                    self.picker(UiLanguage::ALL.to_vec(), Some(p.ui_language), |value| {
                        Message::Preference(Preference::UiLanguage(value))
                    }),
                    width,
                ),
                self.setting(
                    "Compact mode",
                    Some("Smaller rows in Up next, results, collections and Favorites.".into()),
                    toggle(
                        p.ui_density == UiDensity::Compact,
                        Message::Preference(Preference::UiDensity(
                            if p.ui_density == UiDensity::Compact {
                                UiDensity::Comfortable
                            } else {
                                UiDensity::Compact
                            },
                        )),
                    ),
                    width,
                ),
                self.setting(
                    "Reduce motion",
                    None,
                    toggle(
                        p.reduced_motion,
                        Message::Preference(Preference::ReducedMotion(!p.reduced_motion)),
                    ),
                    width,
                ),
                self.setting(
                    "Scale with window",
                    Some("Keeps the layout's proportions on large windows.".into()),
                    toggle(
                        p.scale_with_window,
                        Message::Preference(Preference::ScaleWithWindow(!p.scale_with_window)),
                    ),
                    width,
                ),
                self.setting(
                    "GPU acceleration",
                    Some(
                        "Smoother on large windows; uses much more memory. Applies after restart."
                            .into(),
                    ),
                    toggle(
                        p.gpu_rendering,
                        Message::Preference(Preference::GpuRendering(!p.gpu_rendering)),
                    ),
                    width,
                ),
            ],
        )
    }

    fn playback_box(&self, width: f32) -> Section<'_> {
        let p = &self.preferences;
        self.settings_box(
            "Playback",
            width,
            vec![
                self.setting(
                    "Autoplay",
                    Some("Continue with related songs when the queue ends.".into()),
                    toggle(
                        self.autoplay,
                        Message::Preference(Preference::Autoplay(!self.autoplay)),
                    ),
                    width,
                ),
                self.setting(
                    "Shuffle whole queue",
                    None,
                    toggle(
                        p.shuffle_all,
                        Message::Preference(Preference::ShuffleAll(!p.shuffle_all)),
                    ),
                    width,
                ),
                self.setting(
                    "Restore queue on launch",
                    Some("Playback never starts by itself.".into()),
                    toggle(
                        p.remember_queue,
                        Message::Preference(Preference::RestoreQueue(!p.remember_queue)),
                    ),
                    width,
                ),
            ],
        )
    }

    fn audio_box(&self, width: f32) -> Section<'_> {
        let p = &self.preferences;
        let selected = self
            .output_devices
            .iter()
            .find(|device| device.id.as_ref() == p.audio_output_device_id.as_ref())
            .cloned();
        let device_hint = if let Some(error) = &self.output_devices_error {
            error.clone()
        } else if p.audio_output_device_id.is_some() && selected.is_none() {
            "Saved output is unavailable.".to_owned()
        } else if let Some(active) = &self.playback_status.output_device {
            format!("Active: {active}.")
        } else {
            "Playback switches to it at once.".to_owned()
        };
        // The device list sits under its label at the full width of the box: device names are long.
        let device_width = Length::Fill;
        let device_picker = row![
            pick_list(
                self.output_devices.clone(),
                selected,
                |device: crate::playback::output::OutputDeviceChoice| {
                    Message::Preference(Preference::OutputDevice(device.id))
                }
            )
            .placeholder(self.tr("Default").into_owned())
            .font(MONO)
            .text_size(TEXT_BODY)
            .padding([8, 12])
            .width(device_width)
            .style(theme::pick_style)
            .menu_style(theme::menu_style),
            button(icon("repeat", 14.0, INK))
                .on_press_maybe(
                    (!self.output_devices_loading).then_some(Message::RefreshOutputDevices)
                )
                .padding(9)
                .style(theme::outline_button),
        ]
        .spacing(6)
        .width(Length::Fill);
        self.settings_box(
            "Audio",
            width,
            vec![self.setting_above("Output device", Some(device_hint), device_picker.into())],
        )
    }

    fn general_box(&self, width: f32) -> Section<'_> {
        let p = &self.preferences;
        let mut rows = vec![
            self.setting(
                "Show lyrics by default",
                Some("Now Playing opens on the Lyrics tab.".into()),
                toggle(
                    p.show_lyrics_default,
                    Message::Preference(Preference::ShowLyricsDefault(!p.show_lyrics_default)),
                ),
                width,
            ),
            self.setting(
                "Search suggestions",
                None,
                toggle(
                    p.search_suggestions,
                    Message::Preference(Preference::SearchSuggestions(!p.search_suggestions)),
                ),
                width,
            ),
        ];
        // Notifications and startup need the Linux desktop, so other platforms do not offer them.
        if cfg!(target_os = "linux") {
            rows.push(self.setting(
                "Track notifications",
                Some(
                    "Shows a notification when a track starts while the window is in the background."
                        .into(),
                ),
                toggle(
                    p.track_notifications,
                    Message::Preference(Preference::TrackNotifications(!p.track_notifications)),
                ),
                width,
            ));
        }
        rows.push(self.history_setting(width));
        if cfg!(target_os = "linux") {
            rows.push(self.setting(
                "Launch on startup",
                None,
                toggle(
                    p.startup,
                    Message::Preference(Preference::Startup(!p.startup)),
                ),
                width,
            ));
            rows.push(self.setting(
                "Close to tray",
                Some(if self.tray_available {
                    "Keep playing in the system tray.".into()
                } else {
                    "Needs a system tray host.".into()
                }),
                if self.tray_available {
                    toggle(
                        p.close_to_tray,
                        Message::Preference(Preference::CloseToTray(!p.close_to_tray)),
                    )
                } else {
                    label("—", TEXT_BODY).color(FADED).into()
                },
                width,
            ));
        }
        self.settings_box("General", width, rows)
    }

    fn lyrics_box(&self, width: f32) -> Section<'_> {
        let p = &self.preferences;
        let slider_row =
            |value: String, control: Element<'static, Message>| -> Element<'_, Message> {
                row![
                    container(control).width(150),
                    label(value, TEXT_CAPTION).width(70)
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into()
            };
        self.settings_box(
            "Lyrics",
            width,
            vec![
                self.setting(
                    "Text size",
                    None,
                    slider_row(
                        format!("{} px", p.lyrics_font_size.round() as u32),
                        slider(12.0..=26.0, p.lyrics_font_size, |value| {
                            Message::Preference(Preference::LyricsFontSize(value))
                        })
                        .on_release(Message::PersistState)
                        .step(1.0_f32)
                        .height(theme::SLIDER_H)
                        .style(theme::seek_style)
                        .into(),
                    ),
                    width,
                ),
                self.setting(
                    "Timing offset",
                    Some("Shifts every song. Positive shows lines later.".into()),
                    slider_row(
                        format!("{:+.1} s", p.lyrics_offset_ms as f64 / 1000.0),
                        slider(-5000.0..=5000.0, p.lyrics_offset_ms as f64, |value| {
                            Message::Preference(Preference::LyricsOffset(value.round() as i64))
                        })
                        .on_release(Message::PersistState)
                        .step(100.0)
                        .height(theme::SLIDER_H)
                        .style(theme::seek_style)
                        .into(),
                    ),
                    width,
                ),
                self.setting(
                    "Show translation",
                    None,
                    toggle(
                        p.lyrics_show_translation,
                        Message::Preference(Preference::LyricsTranslation(
                            !p.lyrics_show_translation,
                        )),
                    ),
                    width,
                ),
                self.setting(
                    "Follow current line",
                    None,
                    toggle(
                        p.lyrics_follow,
                        Message::Preference(Preference::LyricsFollow(!p.lyrics_follow)),
                    ),
                    width,
                ),
            ],
        )
    }

    fn integrations_box(&self, width: f32) -> Section<'_> {
        let p = &self.preferences;
        let discord_status = if !p.discord_presence {
            "Off".to_owned()
        } else if self.playback_status.track.is_none()
            || !matches!(
                self.playback_status.state,
                PlaybackState::Playing | PlaybackState::Paused
            )
        {
            crate::discord::ConnectionStatus::Idle.label().to_owned()
        } else {
            self.discord_presence.status().label().to_owned()
        };
        let mut rows = vec![self.setting(
            "Discord presence",
            Some(discord_status),
            toggle(
                p.discord_presence,
                Message::Preference(Preference::DiscordPresence(!p.discord_presence)),
            ),
            width,
        )];
        if cfg!(target_os = "linux") {
            rows.push(self.setting(
                "Media controls (MPRIS)",
                Some(fit(
                    &self.mpris.status(),
                    width - 2.0 * BOX_PADDING - 40.0,
                    TEXT_CAPTION,
                )),
                label("●", TEXT_BODY).into(),
                width,
            ));
        }
        self.settings_box("Integrations", width, rows)
    }

    fn storage_box(&self, width: f32) -> Section<'_> {
        let usage = self.storage_cache_usage;
        let busy = self.storage_loading;
        let language = self.preferences.ui_language;
        let action = |title: &'static str, message: Message| -> Element<'_, Message> {
            button(label(self.tr(title).into_owned(), TEXT_CAPTION))
                .on_press_maybe((!busy).then_some(message))
                .padding([6, 12])
                .style(theme::outline_button)
                .into()
        };
        let mut rows = vec![
            self.setting(
                "Artwork cache",
                Some(
                    self.tr_static("{} on disk · {} in memory")
                        .replacen("{}", &mib(usage.artwork_bytes, language), 1)
                        .replacen("{}", &mib(self.artwork_cache.bytes() as u64, language), 1),
                ),
                action("Clear", Message::ClearArtworkCache),
                width,
            ),
            self.setting(
                "Lyrics cache",
                Some(mib(usage.lyrics_bytes, language)),
                action("Clear", Message::ClearLyricsCache),
                width,
            ),
            self.setting(
                "Playback helpers",
                Some(
                    self.tr_static("Player {} · BotGuard {} · Home {}")
                        .replacen("{}", &mib(usage.player_js_bytes, language), 1)
                        .replacen("{}", &mib(usage.botguard_bytes, language), 1)
                        .replacen("{}", &mib(usage.home_bytes, language), 1),
                ),
                row![
                    action("Player", Message::ClearPlayerJsCache),
                    action("BotGuard", Message::ClearBotGuardCache),
                    action("Home", Message::ClearHomeCache),
                ]
                .spacing(6)
                .into(),
                width,
            ),
            match self.confirmation.as_ref().map(|pending| &pending.kind) {
                Some(Confirmation::Import(snapshot)) => self.setting(
                    "Replace your data with this backup?",
                    Some(self.import_summary(snapshot.as_ref())),
                    self.confirm_buttons("Confirm import", Message::ConfirmImport),
                    width,
                ),
                _ => self.setting(
                    "Local data",
                    Some("Favorites, queue and settings as JSON in Documents.".into()),
                    row![
                        action("Export", Message::ExportLocalData),
                        action("Import", Message::ImportLocalData),
                    ]
                    .spacing(6)
                    .into(),
                    width,
                ),
            },
            self.setting(
                "Diagnostics",
                Some("Sanitized report without tokens or URLs.".into()),
                action("Export", Message::ExportDiagnostics),
                width,
            ),
        ];
        if let Some(notice) = &self.storage_notice {
            rows.push(
                container(
                    text(self.tr_string(notice.clone()))
                        .size(TEXT_CAPTION)
                        .font(MONO)
                        .color(DIM),
                )
                .padding([12, 0])
                .into(),
            );
        }
        if let Some(moved) = &self.state_recovery {
            rows.push(
                container(
                    text(self.tr_string(format!(
                        "Saved settings could not be read and were moved to {moved}"
                    )))
                    .size(TEXT_CAPTION)
                    .font(MONO)
                    .color(ALERT),
                )
                .padding([12, 0])
                .into(),
            );
        }
        self.settings_box("Storage", width, rows)
    }

    fn about_box(&self, width: f32) -> Section<'_> {
        let line = |content: String| -> Element<'_, Message> {
            container(text(content).size(TEXT_CAPTION).font(MONO).color(DIM))
                .padding([10, 0])
                .into()
        };
        self.settings_box(
            "About",
            width,
            vec![
                line(format!(
                    "CrabyTube {} · {}-{}",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )),
                line(self.tr(crate::i18n::KEYS_HELP).into_owned()),
                line(
                    self.tr("GPL-3.0-or-later. Not affiliated with Google or YouTube.")
                        .into_owned(),
                ),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{UiLanguage, masonry_columns, mib};

    #[test]
    fn storage_sizes_use_a_decimal_comma_only_in_spanish() {
        assert_eq!(mib(2_306_867, UiLanguage::English), "2.2 MiB");
        assert_eq!(mib(2_306_867, UiLanguage::Spanish), "2,2 MiB");
        assert_eq!(mib(0, UiLanguage::Spanish), "0,0 MiB");
    }

    #[test]
    fn each_box_goes_to_the_shorter_column_and_ties_go_left() {
        assert_eq!(
            masonry_columns(&[400.0, 272.0, 144.0, 464.0]),
            vec![0, 1, 1, 0]
        );
        assert_eq!(masonry_columns(&[100.0, 100.0]), vec![0, 1]);
        assert_eq!(masonry_columns(&[50.0, 40.0, 40.0]), vec![0, 1, 1]);
        assert!(masonry_columns(&[]).is_empty());
    }
}
