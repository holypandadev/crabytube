//! Window chrome shared by every page: title bar, top tabs and player bar.

use super::theme::{
    self, BLACK, DIM, FADED, GUTTER, INK, MONO, MONO_BOLD, NAV_H, PLAYER_H, TEXT_BODY,
    TEXT_CAPTION, TEXT_SECTION, TEXT_TITLE, TITLE_BAR_H, hrule, icon, label, vrule,
};
use super::{App, Message, Page, Preference, format_duration};
use crate::music::Track;
use crate::playback::PlaybackState;
use crate::preferences::RepeatMode;
use iced::widget::{
    Space, button, column, container, image, mouse_area, responsive, row, sensor, slider, svg, text,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Size};
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

impl App {
    /// The highlighted tab. Now Playing highlights none, so Home reads as a
    /// way back rather than the current page.
    fn active_tab(&self) -> Option<Tab> {
        match self.page {
            Page::Library => Some(Tab::Favorites),
            Page::Settings => Some(Tab::Settings),
            Page::Home => Some(Tab::Home),
            // Playlists, albums, search results and Now Playing are reached from
            // Home but are not Home; a lit Home tab would hide the way back.
            Page::NowPlaying | Page::Search | Page::CollectionDetail(_) => None,
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
    /// `area` is the whole page layer, including the band under the floating tabs. Pages whose
    /// content scrolls start their scroll content below the tabs and let it pass underneath;
    /// the others are laid out in the area below the tabs.
    fn page_view(&self, area: Size) -> Element<'_, Message> {
        let below = below_nav(area);
        match &self.page {
            Page::Search => self.search_page(area, NAV_H),
            Page::Settings => self.settings_page(area, NAV_H),
            Page::Library => under_tabs(self.favorites_page(below)),
            Page::NowPlaying => under_tabs(self.now_playing_page(below)),
            Page::CollectionDetail(_) => under_tabs(self.collection_content(below)),
            Page::Home => under_tabs(self.home_page(below)),
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

    /// Where the Back button in the navigation row leads: the page a collection was opened from, or
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
        // The reported area is the part below the floating tabs.
        let page = sensor(responsive(move |area| self.page_view(area)))
            .on_show(|size| Message::AreaMeasured(below_nav(size)))
            .on_resize(|size| Message::AreaMeasured(below_nav(size)));
        // The tabs float over the page: only their own boxes are opaque, so scrolled content
        // stays visible around them instead of being cut by a full-width band.
        let content = iced::widget::stack![
            container(page)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding([0.0, gutter_for(self.window_width)]),
            self.top_nav(),
        ]
        .width(Length::Fill)
        .height(Length::Fill);
        let mut body = column![self.title_bar(), content];
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

    fn title_bar(&self) -> Element<'_, Message> {
        let dot = |message: Message, glyph: &'static str| {
            button(
                container(text(glyph).size(TEXT_CAPTION).font(MONO_BOLD))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .on_press(message)
            .width(14)
            .height(14)
            .padding(0)
            .style(|_, status| {
                let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: Some(Background::Color(if hovered { BLACK } else { INK })),
                    text_color: if hovered { INK } else { Color::TRANSPARENT },
                    border: Border {
                        color: INK,
                        width: theme::hairline(),
                        radius: 7.0.into(),
                    },
                    ..Default::default()
                }
            })
        };
        let brand = row![
            svg(logo_handle()).width(20).height(20),
            svg(wordmark_handle())
                .width(WORDMARK_H * WORDMARK_ASPECT)
                .height(WORDMARK_H)
                .style(|_, _| svg::Style { color: Some(INK) }),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let drag = mouse_area(
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(Message::StartDrag)
        .on_double_click(Message::ToggleMaximize);
        let bar = if cfg!(target_os = "macos") {
            row![
                dot(Message::CloseWindow, "×"),
                dot(Message::MinimizeWindow, "–"),
                dot(Message::ToggleMaximize, "+"),
                Space::new().width(10),
                brand,
                drag,
            ]
        } else {
            row![
                brand,
                drag,
                dot(Message::MinimizeWindow, "–"),
                dot(Message::ToggleMaximize, "+"),
                dot(Message::CloseWindow, "×"),
            ]
        };
        column![
            container(bar.spacing(8).align_y(Alignment::Center))
                .height(TITLE_BAR_H)
                .padding([0.0, 14.0])
                .center_y(TITLE_BAR_H),
            hrule(INK),
        ]
        .into()
    }

    fn top_nav(&self) -> Element<'_, Message> {
        let narrow = self.window_width < super::home::NARROW_W;
        // Phone-width windows use the short name for the favorites tab.
        let favorites = if narrow {
            "Favorites"
        } else {
            "Favorite songs"
        };
        let titles = ["Home", favorites, "Settings"];
        let active = self.active_tab();
        let widest = titles
            .iter()
            .map(|title| self.tr(title).chars().count())
            .max()
            .unwrap_or(8) as f32;
        let gutter = gutter_for(self.window_width);
        let back = self.back_target();
        // The Back button sits at the left of this row and an equal space at
        // the right keeps the tabs centred.
        let back_width = if narrow { 40.0 } else { BACK_W };
        let reserved = if back.is_some() {
            2.0 * (back_width + 8.0)
        } else {
            0.0
        };
        // JetBrains Mono advances 0.6 em per glyph. On narrow windows the
        // three tabs share the available width and the labels shrink to fit.
        let room = ((self.window_width - 2.0 * gutter - reserved - 4.0) / 3.0).floor();
        let tab_width = (widest * 14.0 * 0.6 + 56.0).ceil().min(room);
        let label_size = ((tab_width - 12.0) / (widest * 0.6))
            .clamp(10.0, 14.0)
            .floor();
        let tab = |title: &'static str, page: Page, which: Tab| {
            button(
                container(label(
                    fit(&self.tr(title), tab_width - 8.0, label_size),
                    label_size,
                ))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
            )
            .on_press(Message::Navigate(page))
            .width(tab_width)
            .height(40)
            .padding(0)
            .style(theme::tab_style(active == Some(which)))
        };
        let tabs = container(
            row![
                tab("Home", Page::Home, Tab::Home),
                vrule(40.0, INK),
                tab(favorites, Page::Library, Tab::Favorites),
                vrule(40.0, INK),
                tab("Settings", Page::Settings, Tab::Settings),
            ]
            .align_y(Alignment::Center),
        )
        .padding(theme::hairline())
        .style(theme::boxed);
        let back: Element<'_, Message> = match back {
            Some(target) => {
                let content: Element<'_, Message> = if narrow {
                    icon("chev-left", 18.0, INK)
                } else {
                    row![
                        icon("chev-left", 16.0, INK),
                        label(self.tr("Back").into_owned(), TEXT_BODY)
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .into()
                };
                button(
                    container(content)
                        .center_x(Length::Fill)
                        .center_y(Length::Fill),
                )
                .on_press(target)
                .width(back_width)
                .height(42)
                .padding(0)
                .style(theme::outline_button)
                .into()
            }
            None => Space::new().into(),
        };
        container(
            row![
                container(back).width(Length::Fill),
                tabs,
                Space::new().width(Length::Fill),
            ]
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .height(NAV_H)
        .padding([0.0, gutter])
        .center_y(NAV_H)
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

    /// Shuffle, previous, play/pause, next and repeat at the given icon size.
    pub(super) fn transport(&self, size: f32) -> Element<'_, Message> {
        self.transport_controls(size, true)
    }

    /// Transport buttons; `full` adds shuffle and repeat.
    fn transport_controls(&self, size: f32, full: bool) -> Element<'_, Message> {
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
        let shuffle = control(
            "shuffle",
            small,
            if self.shuffle { INK } else { FADED },
            Some(Message::ToggleShuffle),
        );
        let repeat = control(
            if self.repeat == RepeatMode::One {
                "repeat-one"
            } else {
                "repeat"
            },
            small,
            if self.repeat == RepeatMode::Off {
                FADED
            } else {
                INK
            },
            Some(Message::CycleRepeat),
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
        let total = self.playback_status.duration.unwrap_or_default();
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
                    self.artist_link(&subtitle, text_width, 12.0, DIM),
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
        let status = match self.playback_status.state {
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
        let muted = self.preferences.muted;
        let mut right = row![
            button(icon(if muted { "volume-off" } else { "volume" }, 18.0, INK))
                .on_press(Message::Preference(Preference::Muted(!muted)))
                .padding(7)
                .style(theme::icon_button),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if !compact {
            right = right.push(
                slider(0.0..=1.0, self.preferences.volume, Message::SetVolume)
                    .on_release(Message::PersistState)
                    .step(0.01_f32)
                    .width(VOLUME_W)
                    .height(theme::SLIDER_H)
                    .style(theme::seek_style),
            );
        }
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

const VOLUME_W: f32 = 104.0;

/// Width of the Back button in the navigation row.
const BACK_W: f32 = 112.0;

/// A page laid out below the floating tab row.
fn under_tabs<'a>(page: Element<'a, Message>) -> Element<'a, Message> {
    column![Space::new().height(NAV_H), page].into()
}

/// The page area below the floating tab row.
fn below_nav(area: Size) -> Size {
    Size::new(area.width, (area.height - NAV_H).max(0.0))
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

#[allow(dead_code)]
pub(super) fn spacer<'a>(height: f32) -> Element<'a, Message> {
    Space::new().height(height).into()
}

#[cfg(test)]
mod tests {
    use super::{display_title, fit, meta_text};

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
