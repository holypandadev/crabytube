//! Window chrome shared by every page: title bar, top tabs and player bar.

use super::theme::{
    self, DIM, FADED, GUTTER, INK, MONO, MONO_BOLD, PLAYER_H, RULE, TEXT_BODY, TEXT_CAPTION,
    TEXT_SECTION, TEXT_TITLE, TITLE_BAR_H, hrule, icon, label, vrule,
};
use super::{App, Message, Page, Preference, format_duration};
use crate::music::Track;
use crate::playback::PlaybackState;
use crate::preferences::RepeatMode;
use iced::widget::{
    Space, button, column, container, image, mouse_area, responsive, row, sensor, slider, svg, text,
};
use iced::{Alignment, Color, Element, Length, Size};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Shortens `value` to fit `width` pixels at `size`. JetBrains Mono advances 0.6 em per cell, so a
/// cell budget is exact for Latin text. Wide characters take two cells, and a grapheme cluster (an
/// accented letter, an emoji sequence) is never split, so the ellipsis follows a whole character.
pub(super) fn fit(value: &str, width: f32, size: f32) -> String {
    let budget = (width / (size * 0.6)).floor().max(1.0) as usize;
    if value.width() <= budget {
        return value.to_owned();
    }
    let keep = budget.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for grapheme in value.graphemes(true) {
        let cells = grapheme.width();
        if used + cells > keep {
            break;
        }
        out.push_str(grapheme);
        used += cells;
    }
    out.push('…');
    out
}

/// Catalog metadata separates its fields with "•"; the interface writes " · " between them. Only
/// the shown text changes: parsing reads the original.
pub(super) fn meta_text(value: &str) -> String {
    value.replace(" • ", " · ").replace('•', "·")
}

/// Tags that uploads add to a title and that say nothing about the song.
const TITLE_TAGS: [&str; 7] = [
    "official audio",
    "official video",
    "official music video",
    "official lyric video",
    "lyric video",
    "lyrics",
    "audio",
];

/// The title as shown: trailing tags such as "(Official Video)" or "[Lyrics]" are dropped, in any
/// case. The stored title, search, MPRIS, Discord and notifications keep the original. A title that
/// would be left empty is shown whole.
pub(super) fn display_title(title: &str) -> &str {
    let mut shown = title.trim_end();
    loop {
        let open = match shown.chars().next_back() {
            Some(')') => '(',
            Some(']') => '[',
            _ => break,
        };
        let Some(start) = shown.rfind(open) else {
            break;
        };
        let tag = shown[start + 1..shown.len() - 1].trim();
        if !TITLE_TAGS
            .iter()
            .any(|known| tag.eq_ignore_ascii_case(known))
        {
            break;
        }
        shown = shown[..start].trim_end();
    }
    if shown.is_empty() { title } else { shown }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Home,
    Favorites,
    Settings,
}

impl Tab {
    /// The tab a page belongs to; pages reached from Home belong to Home.
    fn of_page(page: &Page) -> Self {
        match page {
            Page::Library => Self::Favorites,
            Page::Settings => Self::Settings,
            _ => Self::Home,
        }
    }
}

impl App {
    /// The tab that is underlined, and whether its page is the one on screen. A page reached from a
    /// tab (a playlist, results, Now Playing) keeps that tab underlined in a dimmer colour, so the
    /// header still says where it was opened from.
    fn tab_origin(&self) -> (Tab, bool) {
        match &self.page {
            Page::Home => (Tab::Home, true),
            Page::Library => (Tab::Favorites, true),
            Page::Settings => (Tab::Settings, true),
            Page::NowPlaying => (
                self.now_playing_return
                    .as_ref()
                    .map_or(Tab::Home, Tab::of_page),
                false,
            ),
            Page::Search | Page::CollectionDetail(_) => (Tab::Home, false),
        }
    }

    /// Square cover with a 1 px white outline, or an outlined empty square.
    /// A cover that just arrived fades in over `COVER_FADE`.
    pub(super) fn cover<'a>(
        &self,
        url: Option<&str>,
        size: f32,
        variant: u32,
    ) -> Element<'a, Message> {
        let found = url.and_then(|url| self.artwork_cache.get_for_with_age(url, variant));
        let opacity = match found {
            Some((_, age)) if !self.preferences.reduced_motion => {
                (age.as_secs_f32() / COVER_FADE.as_secs_f32()).clamp(0.0, 1.0)
            }
            _ => 1.0,
        };
        cover_box_faded(found.map(|(handle, _)| handle.clone()), size, opacity)
    }

    /// The page for the area it actually gets, so layout never depends on a
    /// possibly stale window size.
    fn page_view(&self, area: Size) -> Element<'_, Message> {
        match &self.page {
            Page::Search => self.search_page(area),
            Page::Settings => self.settings_page(area),
            Page::Library => self.favorites_page(area),
            Page::NowPlaying => self.now_playing_page(area),
            Page::CollectionDetail(_) => self.collection_content(area),
            Page::Home => self.home_page(area),
        }
    }

    /// An artist name that searches for that artist when pressed. Plain text when there is no name.
    pub(super) fn artist_link(
        &self,
        artist: &str,
        width: f32,
        size: f32,
        color: Color,
    ) -> Element<'_, Message> {
        let shown = fit(artist, width, size);
        if artist.trim().is_empty() {
            return text(shown).size(size).font(MONO).color(color).into();
        }
        button(text(shown).size(size).font(MONO))
            .on_press(Message::SearchFor(artist.to_owned()))
            .padding(0)
            .style(theme::text_link(color))
            .into()
    }

    /// Where Back leads beyond the page history: the page a collection was opened from, or
    /// the page Now Playing was opened from. `None` when there is nowhere to go.
    pub(super) fn back_target(&self) -> Option<Message> {
        match &self.page {
            Page::CollectionDetail(_) => (!self.history.is_empty()).then_some(Message::Back),
            Page::NowPlaying => self
                .now_playing_return
                .as_ref()
                .map(|_| Message::CloseNowPlaying),
            _ => None,
        }
    }

    pub(super) fn shell_view(&self) -> Element<'_, Message> {
        // The sensor reports the measured area to `update`, which sizes
        // artwork requests, carousel steps and virtual lists from it.
        let page = sensor(responsive(move |area| self.page_view(area)))
            .on_show(Message::AreaMeasured)
            .on_resize(Message::AreaMeasured);
        let gutter = gutter_for(self.window_width);
        let content = container(page)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding {
                top: PAGE_TOP,
                right: gutter,
                bottom: 0.0,
                left: gutter,
            });
        let mut body = column![self.header(), content];
        // The player bar appears once something is loaded.
        if !matches!(self.page, Page::NowPlaying) && self.shown_track().is_some() {
            body = body.push(hrule(INK)).push(
                container(responsive(move |size| self.player_bar(size.width)))
                    .width(Length::Fill)
                    .height(PLAYER_H),
            );
        }
        let framed = container(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(theme::hairline())
            .style(theme::frame);
        iced::widget::stack![framed, resize_grips()].into()
    }

    /// The header: one bar with the brand on the left, Back, the tabs and Forward centred on the window
    /// and the window buttons on the right (on macOS they come first, on the left). The empty parts
    /// move the window and a double click maximizes it. When the three zones do not fit side by
    /// side, the wordmark goes and the tabs take a second row of their own.
    fn header(&self) -> Element<'_, Message> {
        let inline = self.tabs_fit_inline();
        let narrow = self.window_width < super::home::NARROW_W;
        let mut brand = row![svg(logo_handle()).width(LOGO_W).height(LOGO_W)]
            .spacing(10)
            .align_y(Alignment::Center);
        if !narrow {
            brand = brand.push(
                svg(wordmark_handle())
                    .width(WORDMARK_H * WORDMARK_ASPECT)
                    .height(WORDMARK_H)
                    .style(|_, _| svg::Style { color: Some(INK) }),
            );
        }
        let drag = || {
            mouse_area(
                container(Space::new())
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .on_press(Message::StartDrag)
            .on_double_click(Message::ToggleMaximize)
        };
        let mut left = row![].height(HEADER_H).align_y(Alignment::Center);
        let mut right = row![].height(HEADER_H).align_y(Alignment::Center);
        if cfg!(target_os = "macos") {
            left = left.push(window_buttons(true));
        }
        left = left.push(Space::new().width(BRAND_INSET)).push(brand);
        left = left.push(drag());
        right = right.push(drag());
        if !cfg!(target_os = "macos") {
            right = right.push(window_buttons(false));
        }
        let mut bar = row![container(left).width(Length::Fill)];
        if inline {
            bar = bar.push(self.tab_row(false));
        }
        bar = bar.push(container(right).width(Length::Fill));
        let mut header = column![bar.height(HEADER_H)];
        if !inline {
            header = header.push(hrule(RULE)).push(self.tab_row(true));
        }
        header.push(hrule(INK)).into()
    }

    /// Whether the tabs fit in the bar between the two side zones, which are equal so that the
    /// tabs sit at the centre of the window.
    fn tabs_fit_inline(&self) -> bool {
        if self.window_width < super::home::NARROW_W {
            return false;
        }
        let buttons = 3.0 * (HEADER_H + 1.0);
        let brand = BRAND_INSET + LOGO_W + 10.0 + WORDMARK_H * WORDMARK_ASPECT;
        let (left, right) = if cfg!(target_os = "macos") {
            (buttons + brand, 0.0)
        } else {
            (brand, buttons)
        };
        let tabs: f32 = self
            .tab_labels(false)
            .iter()
            .map(|title| tab_width(title))
            .sum();
        self.window_width >= 2.0 * left.max(right) + tabs + 2.0 * SIDE_GAP
    }

    /// The three tab labels in the interface language. The second row uses the short name for
    /// Favorites.
    fn tab_labels(&self, second_row: bool) -> [String; 3] {
        let favorites = if second_row {
            "Favorites"
        } else {
            "Favorite songs"
        };
        ["Home", favorites, "Settings"].map(|title| self.tr(title).into_owned())
    }

    /// The tabs as text with an underline: in the bar each is as wide as its label, in the second
    /// row they share the width equally.
    fn tab_row(&self, second_row: bool) -> Element<'_, Message> {
        let (origin, current) = self.tab_origin();
        let height = if second_row { TAB_ROW_H } else { HEADER_H };
        let labels = self.tab_labels(second_row);
        let tabs = [
            (Page::Home, Tab::Home),
            (Page::Library, Tab::Favorites),
            (Page::Settings, Tab::Settings),
        ]
        .into_iter()
        .zip(labels)
        .map(|((page, which), title)| {
            let marked = origin == which;
            let underline = match (marked, current) {
                (true, true) => INK,
                (true, false) => DIM,
                (false, _) => Color::TRANSPARENT,
            };
            let width = if second_row {
                Length::Fill
            } else {
                Length::Fixed(tab_width(&title))
            };
            button(column![
                container(label(title, TEXT_BODY))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
                container(Space::new())
                    .width(Length::Fill)
                    .height(TAB_UNDERLINE)
                    .style(theme::filled(underline)),
            ])
            .on_press(Message::Navigate(page))
            .width(width)
            .height(height)
            .padding(0)
            .style(theme::header_tab(marked && current))
            .into()
        });
        let row = iced::widget::Row::with_children(tabs).height(height);
        let row = if second_row {
            row.width(Length::Fill)
        } else {
            row
        };
        // Back and Forward frame the tabs. Each is faint while there is nowhere to go.
        let back = self
            .back_target()
            .or_else(|| (!self.history.is_empty()).then_some(Message::Back));
        let forward = (!self.forward.is_empty()).then_some(Message::Forward);
        let mut line = row![self.nav_arrow("chev-left", back, height), row];
        line = line.push(self.nav_arrow("chev-right", forward, height));
        if second_row {
            line.width(Length::Fill).into()
        } else {
            line.into()
        }
    }

    /// A Back or Forward arrow beside the tabs, as tall as their row. Without a page to go to it is
    /// drawn faint and does nothing.
    fn nav_arrow(
        &self,
        name: &'static str,
        message: Option<Message>,
        height: f32,
    ) -> Element<'_, Message> {
        let color = if message.is_some() {
            INK
        } else {
            theme::ICON_IDLE
        };
        button(
            container(icon(name, 16.0, color))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .on_press_maybe(message)
        .width(HEADER_H)
        .height(height)
        .padding(0)
        .style(theme::icon_button)
        .into()
    }

    pub(super) fn is_playing(&self) -> bool {
        matches!(
            self.playback_status.state,
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking
        )
    }

    pub(super) fn can_toggle_playback(&self) -> bool {
        self.playback_status.track.is_some() || self.queue_index.is_some()
    }

    /// The track the player bar and Now Playing show. Until the engine reports one, the queue's
    /// current entry stands in, so a restored session shows its track paused and starts nothing.
    pub(super) fn shown_track(&self) -> Option<&Track> {
        self.playback_status
            .track
            .as_ref()
            .or_else(|| self.queue_index.and_then(|index| self.queue.get(index)))
    }

    /// Transport buttons; `full` adds shuffle and repeat.
    pub(super) fn transport_controls(&self, size: f32, full: bool) -> Element<'_, Message> {
        let can_prev = self.prev_queue_index().is_some() || self.playback_status.track.is_some();
        let can_next = self.next_queue_index().is_some();
        let small = (size * 0.8).round();
        let control =
            |name: &'static str, icon_size: f32, color: Color, message: Option<Message>| {
                button(icon(name, icon_size, color))
                    .on_press_maybe(message)
                    .padding(7)
                    .style(theme::icon_button)
            };
        // A toggle that is on is white and boxed; off, it is grey and bare, so its state does not
        // rest on the grey alone.
        let toggle = |name: &'static str, on: bool, message: Message| {
            button(icon(name, small, if on { INK } else { FADED }))
                .on_press(message)
                .padding(7)
                .style(if on {
                    theme::icon_button_active
                } else {
                    theme::icon_button
                })
        };
        let shuffle = toggle("shuffle", self.shuffle, Message::ToggleShuffle);
        let repeat = toggle(
            self.repeat_icon(),
            self.repeat != RepeatMode::Off,
            Message::CycleRepeat,
        );
        let mut controls = row![];
        if full {
            controls = controls.push(shuffle);
        }
        controls = controls
            .push(control(
                "prev",
                size,
                if can_prev { INK } else { FADED },
                can_prev.then_some(Message::PrevTrack),
            ))
            .push(control(
                if self.is_playing() { "pause" } else { "play" },
                size + 6.0,
                INK,
                self.can_toggle_playback()
                    .then_some(Message::TogglePlayback),
            ))
            .push(control(
                "next",
                size,
                if can_next { INK } else { FADED },
                can_next.then_some(Message::NextTrack),
            ));
        if full {
            controls = controls.push(repeat);
        }
        controls.spacing(14).align_y(Alignment::Center).into()
    }

    fn repeat_icon(&self) -> &'static str {
        if self.repeat == RepeatMode::One {
            "repeat-one"
        } else {
            "repeat"
        }
    }

    /// The mute button and, with `rail`, a volume rail of that length beside it. The player bar and
    /// Now Playing share it.
    pub(super) fn volume_control(&self, rail: Option<f32>) -> Element<'_, Message> {
        let muted = self.preferences.muted;
        let mut control = row![
            button(icon(if muted { "volume-off" } else { "volume" }, 18.0, INK))
                .on_press(Message::Preference(Preference::Muted(!muted)))
                .padding(7)
                .style(theme::icon_button),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if let Some(rail) = rail {
            control = control.push(
                slider(0.0..=1.0, self.preferences.volume, Message::SetVolume)
                    .on_release(Message::PersistState)
                    .step(0.01_f32)
                    .width(rail)
                    .height(theme::SLIDER_H)
                    .style(theme::seek_style),
            );
        }
        control.into()
    }

    /// Elapsed time, seek rail and total time. A live broadcast has no rail and no
    /// total: it shows the elapsed time and a LIVE marker instead.
    pub(super) fn seek_row(&self, time_size: f32) -> Element<'_, Message> {
        if self.playback_status.live {
            return row![
                label(
                    format_duration(std::time::Duration::from_secs_f32(
                        self.playback_status.position.as_secs_f32()
                    )),
                    time_size
                )
                .color(DIM),
                Space::new().width(Length::Fill),
                text("● LIVE").size(time_size).font(MONO_BOLD).color(INK),
            ]
            .align_y(Alignment::Center)
            .into();
        }
        let max_position = self
            .playback_status
            .duration
            .map(|duration| duration.as_secs_f32())
            .unwrap_or(0.0);
        let shown = self
            .seek_preview
            .unwrap_or(self.playback_status.position.as_secs_f32())
            .clamp(0.0, max_position.max(0.0));
        // A restored track has no stream yet; its known length is shown until it plays.
        let total = self
            .playback_status
            .duration
            .or_else(|| self.shown_track().and_then(|track| self.duration_of(track)))
            .unwrap_or_default();
        row![
            label(
                format_duration(std::time::Duration::from_secs_f32(shown)),
                time_size
            )
            .color(DIM),
            slider(0.0..=max_position.max(0.1), shown, Message::SeekChanged)
                .on_release(Message::SeekReleased)
                .step(0.1_f32)
                .width(Length::Fill)
                .height(theme::SLIDER_H)
                .style(theme::seek_style),
            label(format_duration(total), time_size).color(DIM),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    }

    pub(super) fn like_button(&self, video_id: Option<&str>, size: f32) -> Element<'_, Message> {
        let liked = video_id.is_some_and(|id| self.liked.contains(id));
        button(icon(if liked { "like-filled" } else { "like" }, size, INK))
            .on_press_maybe(video_id.map(|id| Message::ToggleLike(id.to_owned())))
            .padding(6)
            .style(theme::icon_button)
            .into()
    }

    /// Phone-width player: a full-width seek rail on top, then the cover,
    /// title and the three main transport buttons.
    /// The mini player's second line: the artist, or the seek notice for a moment, as in the bar.
    fn mini_subtitle(&self, artist: &str, width: f32) -> Element<'_, Message> {
        if self.seek_notice.is_some() {
            let notice = self.tr("Couldn't seek there");
            return text(fit(&notice, width, TEXT_CAPTION))
                .size(TEXT_CAPTION)
                .font(MONO)
                .color(DIM)
                .into();
        }
        self.artist_link(artist, width, 12.0, DIM)
    }

    fn mini_player(&self, width: f32) -> Element<'_, Message> {
        let track = self.shown_track();
        let max_position = self
            .playback_status
            .duration
            .map(|duration| duration.as_secs_f32())
            .unwrap_or(0.0);
        let shown = self
            .seek_preview
            .unwrap_or(self.playback_status.position.as_secs_f32())
            .clamp(0.0, max_position.max(0.0));
        let rail: Element<'_, Message> = if self.playback_status.live {
            // A live broadcast has no length to seek through.
            Space::new()
                .width(Length::Fill)
                .height(theme::SLIDER_H)
                .into()
        } else {
            slider(0.0..=max_position.max(0.1), shown, Message::SeekChanged)
                .on_release(Message::SeekReleased)
                .step(0.1_f32)
                .width(Length::Fill)
                .height(theme::SLIDER_H)
                .style(theme::seek_style)
                .into()
        };
        let text_width = (width - 32.0 - 44.0 - 12.0 - 150.0).max(60.0);
        let title = track.map_or_else(String::new, |t| display_title(&t.title).to_owned());
        let subtitle = track.map_or_else(String::new, |t| t.artist.clone());
        // A mouse area rather than a button: the artist name inside is a button of its own,
        // and a click on it searches instead of opening Now Playing.
        let now = mouse_area(
            row![
                self.cover(
                    track.and_then(|t| t.thumbnail_url.as_deref()),
                    44.0,
                    crate::artwork::THUMB_MAX
                ),
                column![
                    text(fit(&title, text_width, TEXT_BODY))
                        .size(TEXT_BODY)
                        .font(MONO),
                    self.mini_subtitle(&subtitle, text_width),
                ]
                .spacing(4)
                .width(Length::Fill),
            ]
            .spacing(12)
            .width(Length::Fill)
            .align_y(Alignment::Center),
        )
        .interaction(iced::mouse::Interaction::Pointer);
        let now = match track {
            Some(_) => now.on_press(Message::OpenNowPlaying),
            None => now,
        };
        column![
            container(rail).padding([0, 8]),
            container(
                row![now, self.transport_controls(18.0, false)]
                    .spacing(8)
                    .align_y(Alignment::Center),
            )
            .height(Length::Fill)
            .padding([0, 16])
            .center_y(Length::Fill),
        ]
        .height(PLAYER_H)
        .into()
    }

    fn player_bar(&self, width: f32) -> Element<'_, Message> {
        if width < super::home::NARROW_W {
            return self.mini_player(width);
        }
        // Below this width the bar drops shuffle, repeat and the volume rail.
        let compact = width < 760.0;
        let side = if compact {
            (width * 0.36).clamp(120.0, 300.0)
        } else {
            (width * 0.28).clamp(150.0, 420.0)
        };
        let track = self.shown_track();
        // A seek that failed shows its notice for a moment, ahead of the playback state.
        let status = if self.seek_notice.is_some() {
            Some(self.tr("Couldn't seek there").into_owned())
        } else {
            match self.playback_status.state {
                PlaybackState::Resolving => Some(self.tr("Loading…").into_owned()),
                PlaybackState::Buffering => Some(self.tr("Buffering…").into_owned()),
                PlaybackState::Error => Some(
                    self.tr(self
                        .playback_status
                        .error
                        .as_deref()
                        .unwrap_or("Playback failed"))
                        .into_owned(),
                ),
                _ => None,
            }
        };
        let text_width = (side - 52.0 - 12.0 - 44.0 - 20.0).max(40.0);
        let title = track.map_or_else(
            || self.tr("Nothing playing").into_owned(),
            |t| display_title(&t.title).to_owned(),
        );
        let subtitle = status.unwrap_or_else(|| {
            track.map_or_else(
                || self.tr("Search to start").into_owned(),
                |t| t.artist.clone(),
            )
        });
        let now = button(
            row![
                self.cover(
                    track.and_then(|t| t.thumbnail_url.as_deref()),
                    52.0,
                    crate::artwork::THUMB_MAX
                ),
                column![
                    text(fit(&title, text_width, TEXT_BODY))
                        .size(TEXT_BODY)
                        .font(MONO),
                    text(fit(&subtitle, text_width, TEXT_CAPTION))
                        .size(TEXT_CAPTION)
                        .font(MONO)
                        .color(DIM),
                ]
                .spacing(4),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .on_press_maybe(track.map(|_| Message::OpenNowPlaying))
        .padding(0)
        .style(theme::bare_button);
        let left = row![
            now,
            self.like_button(track.map(|t| t.video_id.as_str()), 18.0)
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let center = column![
            container(self.transport_controls(18.0, !compact)).center_x(Length::Fill),
            self.seek_row(TEXT_CAPTION),
        ]
        .spacing(2)
        .width(Length::Fill);
        let right = self.volume_control((!compact).then_some(VOLUME_W));
        let right_width = if compact { 44.0 } else { side };
        container(
            row![
                container(left).width(side),
                center,
                container(right).width(right_width).align_right(right_width),
            ]
            .spacing(if compact { 12 } else { 24 })
            .align_y(Alignment::Center),
        )
        .height(PLAYER_H)
        .padding([0, 16])
        .center_y(PLAYER_H)
        .into()
    }
}

pub(super) const VOLUME_W: f32 = 104.0;

/// The header bar: tabs, Back and the window buttons are this tall.
pub(super) const HEADER_H: f32 = TITLE_BAR_H;
/// The second row that holds the tabs when they do not fit in the bar.
pub(super) const TAB_ROW_H: f32 = 36.0;
/// Space between the header and the top of every page.
const PAGE_TOP: f32 = 20.0;
const TAB_UNDERLINE: f32 = 2.0;
/// Room on each side of a tab label in the bar.
const TAB_PADDING: f32 = 16.0;
const LOGO_W: f32 = 20.0;
const BRAND_INSET: f32 = 14.0;
/// The least room left between the tabs and each side zone.
const SIDE_GAP: f32 = 8.0;

/// Width of a tab in the bar: its label and the padding on both sides.
fn tab_width(title: &str) -> f32 {
    (title.chars().count() as f32 * TEXT_BODY * 0.6 + 2.0 * TAB_PADDING).ceil()
}

/// Minimize, maximize and close as square buttons the height of the bar, each with a rule on
/// the side that faces the page. macOS puts close first.
fn window_buttons<'a>(macos: bool) -> Element<'a, Message> {
    let button_for = |glyph: &'static str, message: Message| {
        button(theme::window_icon(glyph))
            .on_press(message)
            .width(HEADER_H)
            .height(HEADER_H)
            .padding(0)
            .style(theme::window_button)
    };
    let buttons = if macos {
        [
            ("close", Message::CloseWindow),
            ("minus", Message::MinimizeWindow),
            ("square", Message::ToggleMaximize),
        ]
    } else {
        [
            ("minus", Message::MinimizeWindow),
            ("square", Message::ToggleMaximize),
            ("close", Message::CloseWindow),
        ]
    };
    let mut row = row![].height(HEADER_H);
    for (glyph, message) in buttons {
        if macos {
            row = row
                .push(button_for(glyph, message))
                .push(vrule(HEADER_H, RULE));
        } else {
            row = row
                .push(vrule(HEADER_H, RULE))
                .push(button_for(glyph, message));
        }
    }
    row.into()
}

/// Side margin of the page: narrower on phone-sized windows.
pub(super) fn gutter_for(window_width: f32) -> f32 {
    if window_width < 640.0 { 16.0 } else { GUTTER }
}

/// The "CRABYTUBE" wordmark: geometric block letters drawn as paths
/// (`assets/crabytube-wordmark.svg`, viewBox 74.2 × 7).
const WORDMARK_H: f32 = 11.0;
const WORDMARK_ASPECT: f32 = 74.2 / 7.0;

fn wordmark_handle() -> svg::Handle {
    use std::sync::LazyLock;
    static WORDMARK: LazyLock<svg::Handle> = LazyLock::new(|| {
        svg::Handle::from_memory(include_bytes!("../../assets/crabytube-wordmark.svg").as_slice())
    });
    WORDMARK.clone()
}

/// Thickness of the invisible strips along the window edges that resize it.
const GRIP: f32 = 6.0;

/// Edge and corner strips that start an interactive resize. The window draws
/// its own frame, so the compositor offers no resize border of its own. The
/// middle stays empty, so everything else receives the pointer as usual.
fn resize_grips<'a>() -> Element<'a, Message> {
    use iced::mouse::Interaction;
    use iced::window::Direction;
    let grip = |direction: Direction, interaction: Interaction, width: Length, height: Length| {
        mouse_area(Space::new().width(width).height(height))
            .interaction(interaction)
            .on_press(Message::StartResize(direction))
    };
    let corner = |direction, interaction| {
        grip(
            direction,
            interaction,
            Length::Fixed(GRIP * 2.0),
            Length::Fixed(GRIP * 2.0),
        )
    };
    column![
        row![
            corner(Direction::NorthWest, Interaction::ResizingDiagonallyDown),
            grip(
                Direction::North,
                Interaction::ResizingVertically,
                Length::Fill,
                Length::Fixed(GRIP)
            ),
            corner(Direction::NorthEast, Interaction::ResizingDiagonallyUp),
        ],
        row![
            grip(
                Direction::West,
                Interaction::ResizingHorizontally,
                Length::Fixed(GRIP),
                Length::Fill
            ),
            Space::new().width(Length::Fill).height(Length::Fill),
            grip(
                Direction::East,
                Interaction::ResizingHorizontally,
                Length::Fixed(GRIP),
                Length::Fill
            ),
        ]
        .height(Length::Fill),
        row![
            corner(Direction::SouthWest, Interaction::ResizingDiagonallyUp),
            grip(
                Direction::South,
                Interaction::ResizingVertically,
                Length::Fill,
                Length::Fixed(GRIP)
            ),
            corner(Direction::SouthEast, Interaction::ResizingDiagonallyDown),
        ],
    ]
    .into()
}

fn logo_handle() -> svg::Handle {
    use std::sync::LazyLock;
    static LOGO: LazyLock<svg::Handle> = LazyLock::new(|| {
        svg::Handle::from_memory(include_bytes!("../../assets/crabytube-logo.svg").as_slice())
    });
    LOGO.clone()
}

/// How long a newly arrived cover takes to fade in.
pub(super) const COVER_FADE: std::time::Duration = std::time::Duration::from_millis(260);

fn cover_box_faded<'a>(
    handle: Option<image::Handle>,
    size: f32,
    opacity: f32,
) -> Element<'a, Message> {
    let line = theme::hairline();
    let inner: Element<'a, Message> = match handle {
        Some(handle) => image(handle)
            .width(size - 2.0 * line)
            .height(size - 2.0 * line)
            .content_fit(iced::ContentFit::Cover)
            .opacity(opacity)
            .into(),
        None => container(
            text("◻")
                .size((size / 4.0).clamp(TEXT_CAPTION, TEXT_TITLE))
                .font(MONO)
                .color(FADED),
        )
        .width(size - 2.0 * line)
        .height(size - 2.0 * line)
        .center_x(size - 2.0 * line)
        .center_y(size - 2.0 * line)
        .into(),
    };
    container(inner)
        .width(size)
        .height(size)
        .padding(theme::hairline())
        .style(theme::boxed)
        .into()
}

/// Section heading followed by a 1 px rule that fills the remaining width.
pub(super) fn section_heading<'a>(
    title: String,
    trailing: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut line = row![
        text(title).size(TEXT_SECTION).font(MONO),
        container(hrule(INK))
            .width(Length::Fill)
            .center_y(Length::Fixed(20.0)),
    ]
    .spacing(16)
    .align_y(Alignment::Center);
    if let Some(trailing) = trailing {
        line = line.push(trailing);
    }
    line.into()
}

#[cfg(test)]
mod tests {
    use super::super::{App, Page};
    use super::{Tab, display_title, fit, meta_text};

    #[test]
    fn the_tabs_sit_in_the_bar_on_wide_windows_and_take_a_second_row_on_narrow_ones() {
        let at = |window_width| App {
            window_width,
            ..App::default()
        };
        assert!(at(1418.0).tabs_fit_inline());
        assert!(at(1270.0).tabs_fit_inline());
        assert!(!at(390.0).tabs_fit_inline());
        assert!(!at(360.0).tabs_fit_inline());
    }

    #[test]
    fn a_page_opened_from_a_tab_keeps_that_tab_marked_as_its_origin() {
        let mut app = App {
            page: Page::Library,
            ..App::default()
        };
        assert!(app.tab_origin() == (Tab::Favorites, true));
        app.page = Page::NowPlaying;
        app.now_playing_return = Some(Page::Library);
        assert!(app.tab_origin() == (Tab::Favorites, false));
        app.page = Page::Search;
        assert!(app.tab_origin() == (Tab::Home, false));
    }

    #[test]
    fn upload_tags_are_dropped_from_the_shown_title_only_at_its_end() {
        assert_eq!(display_title("Song (Official Video)"), "Song");
        assert_eq!(display_title("Song [LYRICS]"), "Song");
        assert_eq!(display_title("Song (Official Audio) [Lyric Video]"), "Song");
        assert_eq!(display_title("Song (Live)"), "Song (Live)");
        assert_eq!(
            display_title("Audio (Lyrics) Remix"),
            "Audio (Lyrics) Remix"
        );
        assert_eq!(display_title("(Official Video)"), "(Official Video)");
        assert_eq!(
            display_title("Song (Remastered 2009)"),
            "Song (Remastered 2009)"
        );
    }

    #[test]
    fn catalog_metadata_is_shown_with_middle_dots() {
        assert_eq!(meta_text("Song • Artist • 3:45"), "Song · Artist · 3:45");
        assert_eq!(meta_text("Album"), "Album");
    }

    #[test]
    fn latin_text_is_cut_by_characters_with_an_ellipsis() {
        // 72 px at 12 px is ten cells of 7.2 px: nine characters and the ellipsis.
        assert_eq!(fit("Never Gonna Give You Up", 72.0, 12.0), "Never Gon…");
        assert_eq!(fit("Short", 72.0, 12.0), "Short");
    }

    #[test]
    fn wide_characters_take_two_cells() {
        // 73 px is ten cells: four of the eight wide characters fit beside the ellipsis.
        assert_eq!(fit("日本語の曲名です", 73.0, 12.0), "日本語の…");
    }

    #[test]
    fn a_grapheme_cluster_is_kept_whole() {
        // "e" with a combining accent is one character of one cell, so it is kept with its accent.
        assert_eq!(fit("cafe\u{301} au lait", 36.0, 12.0), "cafe\u{301}…");
    }
}
