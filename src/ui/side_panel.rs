//! The panel down the left of the window that takes the place of the header's tabs when chosen in
//! Settings: Home, Library and Settings, and under them the user's lists. On a window too narrow
//! for its names it becomes a rail of icons and covers, and on a phone-width window it gives way to
//! the tab bar at the foot.

use super::layout::virtual_range;
use super::messages::ListId;
use super::theme::{
    self, DIM, HOVER, INK, MONO, RULE, TEXT_BODY, TEXT_CAPTION, hrule, icon, label,
};
use super::{App, Message, Page};
use crate::preferences::{LibrarySort, Navigation};
use iced::widget::{Space, button, column, container, row, scrollable, text, tooltip};
use iced::{Alignment, Background, Border, Color, Element, Length, Theme};

/// Below this window width the panel shows icons and covers only.
pub(super) const RAIL_FROM: f32 = 960.0;
/// The width of the rail of icons and covers.
pub(super) const RAIL_W: f32 = 72.0;
/// The panel takes this share of the window, between the two widths after it.
const PANEL_SHARE: f32 = 0.19;
const PANEL_MIN_W: f32 = 250.0;
const PANEL_MAX_W: f32 = 300.0;
/// A row of Home, Library or Settings, and on the rail, where each is a cell with its icon alone.
const NAV_H: f32 = 40.0;
const RAIL_NAV_H: f32 = 48.0;
/// The icon of a row of the panel, and on the rail.
const NAV_ICON: f32 = 18.0;
const RAIL_NAV_ICON: f32 = 20.0;
/// Space above and below the row at the panel's foot that collapses or expands it.
const FOOT_GAP: f32 = 8.0;
/// A row of a list: its cover with the name and the number of songs beside it.
pub(super) const LIST_ROW_H: f32 = 56.0;
const LIST_COVER: f32 = 40.0;
/// Space between a list's name and its number of songs, as on Home.
const TWO_LINES_GAP: f32 = 3.0;
/// The bar at a row's left edge that marks the page on screen.
const MARKER_W: f32 = 2.0;
/// Space around the panel's content.
const PAD_TOP: f32 = 12.0;
pub(super) const PAD_X: f32 = 8.0;
/// Height of the line with "Your lists" and the button that makes a playlist.
const LISTS_HEAD_H: f32 = 32.0;
/// Space above and below the rule between the sections and the lists.
const RULE_GAP: f32 = 12.0;

/// How a row of the panel is marked: the page on screen, a page opened from it, or neither.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Current,
    Origin,
    None,
}

impl Mark {
    fn marker(self) -> Color {
        match self {
            Self::Current => INK,
            Self::Origin => DIM,
            Self::None => Color::TRANSPARENT,
        }
    }
}

/// A row of the panel: grey text, white while hovered or for the section it is in, and filled for
/// the page on screen.
fn panel_row(mark: Mark) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: (hovered || mark == Mark::Current).then_some(Background::Color(HOVER)),
            text_color: if hovered || mark != Mark::None {
                INK
            } else {
                DIM
            },
            border: Border::default(),
            ..Default::default()
        }
    }
}

/// The panel's width for a window as wide as `window_width`, between `PANEL_MIN_W` and
/// `PANEL_MAX_W`, or the rail's below `RAIL_FROM` and while the user keeps it collapsed.
pub(super) fn panel_width(window_width: f32, collapsed: bool) -> f32 {
    if collapsed || window_width < RAIL_FROM {
        RAIL_W
    } else {
        (window_width * PANEL_SHARE)
            .clamp(PANEL_MIN_W, PANEL_MAX_W)
            .floor()
    }
}

impl App {
    /// Whether the panel takes the place of the header's tabs: chosen in Settings, on a window
    /// wider than a phone.
    pub(super) fn side_panel_shown(&self) -> bool {
        self.preferences.navigation == Navigation::SidePanel && !self.mobile()
    }

    /// The width the panel takes from the page, its rule included; 0 without it.
    pub(super) fn side_panel_width(&self) -> f32 {
        if self.side_panel_shown() {
            self.panel_w() + theme::hairline()
        } else {
            0.0
        }
    }

    /// The panel's own width, without its rule.
    pub(super) fn panel_w(&self) -> f32 {
        panel_width(self.window_width, self.preferences.panel_collapsed)
    }

    /// Whether the panel shows icons and covers only: on a narrow window, or collapsed.
    fn side_panel_rail(&self) -> bool {
        self.preferences.panel_collapsed || self.window_width < RAIL_FROM
    }

    /// Whether the window has room for the whole panel, so the button that collapses or expands
    /// it shows.
    fn panel_can_expand(&self) -> bool {
        self.window_width >= RAIL_FROM
    }

    /// The lists of the panel: Favorite songs first, then the playlists in the Library's order.
    pub(super) fn panel_lists(&self) -> Vec<ListId> {
        let mut places: Vec<usize> = (0..self.playlists.len()).collect();
        match self.preferences.library_sort {
            LibrarySort::Custom => {}
            // Ids only grow, so the newest playlist has the largest.
            LibrarySort::Recent => {
                places.sort_by_key(|&place| std::cmp::Reverse(self.playlists[place].id));
            }
            LibrarySort::Name => {
                places.sort_by_cached_key(|&place| self.playlists[place].name.to_lowercase());
            }
        }
        std::iter::once(ListId::Favorites)
            .chain(
                places
                    .into_iter()
                    .map(|place| ListId::Playlist(self.playlists[place].id)),
            )
            .collect()
    }

    /// The covers of the lists the panel shows on screen.
    pub(super) fn panel_artwork(&self) -> Vec<(String, u32)> {
        if !self.side_panel_shown() {
            return Vec::new();
        }
        let lists = self.panel_lists();
        let range = virtual_range(
            lists.len(),
            self.panel_scroll.y,
            self.panel_lists_height(),
            LIST_ROW_H,
            4,
        );
        lists[range]
            .iter()
            .flat_map(|&list| {
                let variant = self.list_cover_variant(list, LIST_COVER);
                self.list_cover_urls(list)
                    .into_iter()
                    .map(move |url| (url.to_owned(), variant))
            })
            .collect()
    }

    /// The height left to the lists, under the header, the sections and the player bar.
    fn panel_lists_height(&self) -> f32 {
        let player = if self.player_bar_shown() {
            theme::PLAYER_H
        } else {
            0.0
        };
        (self.window_height
            - theme::TITLE_BAR_H
            - player
            - PAD_TOP
            - 3.0 * self.nav_height()
            - 2.0 * RULE_GAP
            - self.lists_head_height()
            - self.foot_height())
        .max(0.0)
    }

    /// The height of a row of Home, Library or Settings.
    fn nav_height(&self) -> f32 {
        if self.side_panel_rail() {
            RAIL_NAV_H
        } else {
            NAV_H
        }
    }

    /// The height of the head of the lists: the line with "Your lists", or on the rail the cell
    /// of the button that makes a playlist, as tall as a list's.
    fn lists_head_height(&self) -> f32 {
        if self.side_panel_rail() {
            LIST_ROW_H
        } else {
            LISTS_HEAD_H
        }
    }

    /// The height of the foot, with its rule: the row that collapses or expands the panel, on a
    /// window with room for the whole panel.
    fn foot_height(&self) -> f32 {
        if self.panel_can_expand() {
            theme::hairline() + 2.0 * FOOT_GAP + self.nav_height()
        } else {
            0.0
        }
    }

    /// How the row of Home, Library or Settings is marked.
    fn section_mark(&self, section: &Page) -> Mark {
        let opened_from = match &self.page {
            Page::NowPlaying => self.history.last().cloned().unwrap_or(Page::Home),
            page => page.clone(),
        };
        let belongs = |page: &Page| match page {
            Page::Library | Page::List(_) => Page::Library,
            Page::Settings => Page::Settings,
            _ => Page::Home,
        };
        if belongs(&opened_from) != *section {
            Mark::None
        } else if self.page == *section {
            Mark::Current
        } else {
            Mark::Origin
        }
    }

    /// The panel: Home, Library and Settings, a rule, and the user's lists, which scroll.
    pub(super) fn navigation_panel(&self) -> Element<'_, Message> {
        let rail = self.side_panel_rail();
        let width = self.panel_w();
        let inner = width - 2.0 * PAD_X;
        let nav_icon = if rail { RAIL_NAV_ICON } else { NAV_ICON };
        let mut sections = column![].spacing(0);
        for (page, glyph, name) in [
            (Page::Home, "home", "Home"),
            (Page::Library, "library", "Library"),
            (Page::Settings, "settings", "Settings"),
        ] {
            let mark = self.section_mark(&page);
            let title = self.tr(name).into_owned();
            sections = sections.push(self.panel_entry(
                mark,
                icon(glyph, nav_icon, if mark == Mark::None { DIM } else { INK }),
                (!rail).then(|| label(title.clone(), TEXT_BODY).into()),
                title,
                Message::Navigate(page),
                self.nav_height(),
                inner,
            ));
        }
        // On the rail the button that makes a playlist is a box the size of a cover, in a cell of
        // its own, so it lines up with the lists under it.
        let (new_size, new_icon) = if rail {
            (LIST_COVER, 16.0)
        } else {
            (28.0, 14.0)
        };
        let new = button(
            container(icon("plus", new_icon, INK))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .on_press(Message::NewPlaylist(None))
        .width(new_size)
        .height(new_size)
        .padding(0)
        .style(theme::icon_button_active);
        let new = self.with_tooltip(new.into(), self.tr("New playlist").into_owned());
        let head: Element<'_, Message> = if rail {
            container(new)
                .center_x(Length::Fill)
                .center_y(LIST_ROW_H)
                .height(LIST_ROW_H)
                .into()
        } else {
            row![
                Space::new().width(MARKER_W + 12.0),
                label(self.tr("Your lists").into_owned(), TEXT_BODY),
                Space::new().width(Length::Fill),
                new,
            ]
            .height(LISTS_HEAD_H)
            .align_y(Alignment::Center)
            .into()
        };
        let on_screen = match self.page {
            Page::List(list) => Some(list),
            _ => None,
        };
        let mut lists = column![].spacing(0);
        for list in self.panel_lists() {
            let (name, count) = match list {
                ListId::Favorites => (self.tr("Favorite songs").into_owned(), self.favorites.len()),
                ListId::Playlist(id) => self
                    .playlist(id)
                    .map(|playlist| (playlist.name.clone(), playlist.tracks.len()))
                    .unwrap_or_default(),
            };
            let mark = if on_screen == Some(list) {
                Mark::Current
            } else {
                Mark::None
            };
            // The marker, the gaps and the cover leave the rest of the row to the two lines.
            let text_width = (inner - MARKER_W - 10.0 - LIST_COVER - 12.0 - 8.0).max(40.0);
            let lines: Option<Element<'_, Message>> = (!rail).then(|| {
                column![
                    text(super::shell::fit(&name, text_width, TEXT_BODY))
                        .size(TEXT_BODY)
                        .font(MONO)
                        .color(INK)
                        .wrapping(text::Wrapping::None),
                    label(self.song_count(count, false), TEXT_CAPTION).color(DIM),
                ]
                .spacing(TWO_LINES_GAP)
                .into()
            });
            lists = lists.push(self.panel_entry(
                mark,
                self.list_cover(list, LIST_COVER),
                lines,
                name,
                Message::Navigate(Page::List(list)),
                LIST_ROW_H,
                inner,
            ));
        }
        let lists = scrollable(lists)
            .id("panel-lists")
            .on_scroll(|viewport| Message::PanelListsScrolled(viewport.absolute_offset()))
            .direction(theme::vbar())
            .style(theme::scroll_style)
            .height(Length::Fill);
        let mut panel = column![
            Space::new().height(PAD_TOP),
            sections,
            Space::new().height(RULE_GAP),
            hrule(RULE),
            Space::new().height(RULE_GAP),
            head,
            lists,
        ];
        // The foot: one row, always in the same place, that collapses the panel or expands it.
        if self.panel_can_expand() {
            let (glyph, tip) = if rail {
                ("panel-expand", "Expand the panel")
            } else {
                ("panel-collapse", "Collapse the panel")
            };
            let tip = self.tr(tip).into_owned();
            let text_width = (inner - MARKER_W - 10.0 - NAV_ICON - 12.0 - 8.0).max(40.0);
            let shown = super::shell::fit(&tip, text_width, TEXT_BODY);
            panel = panel
                .push(hrule(RULE))
                .push(Space::new().height(FOOT_GAP))
                .push(self.panel_entry(
                    Mark::None,
                    icon(glyph, nav_icon, DIM),
                    (!rail).then(|| label(shown, TEXT_BODY).into()),
                    tip,
                    Message::Preference(super::Preference::PanelCollapsed(!rail)),
                    self.nav_height(),
                    inner,
                ))
                .push(Space::new().height(FOOT_GAP));
        }
        container(panel)
            .width(width)
            .height(Length::Fill)
            .padding(iced::Padding {
                top: 0.0,
                right: PAD_X,
                bottom: 0.0,
                left: PAD_X,
            })
            .into()
    }

    /// A row of the panel: the marker, a picture, and the text beside it; on the rail, the picture
    /// alone, centred, with its name in a tooltip.
    #[allow(clippy::too_many_arguments)]
    fn panel_entry<'a>(
        &'a self,
        mark: Mark,
        picture: Element<'a, Message>,
        lines: Option<Element<'a, Message>>,
        name: String,
        message: Message,
        height: f32,
        width: f32,
    ) -> Element<'a, Message> {
        let marker = container(Space::new())
            .width(MARKER_W)
            .height(Length::Fill)
            .style(theme::filled(mark.marker()));
        let content: Element<'a, Message> = match lines {
            Some(lines) => row![
                marker,
                Space::new().width(10),
                picture,
                Space::new().width(12),
                lines
            ]
            .align_y(Alignment::Center)
            .into(),
            None => row![
                marker,
                container(picture)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
                Space::new().width(MARKER_W),
            ]
            .into(),
        };
        let rail = self.side_panel_rail();
        // On the rail the picture sits at the centre of the cell, between the marker and an equal
        // space on the other side.
        let entry = button(content)
            .on_press(message)
            .width(width)
            .height(height)
            .padding(iced::Padding {
                top: 0.0,
                right: if rail { 0.0 } else { 8.0 },
                bottom: 0.0,
                left: 0.0,
            })
            .style(panel_row(mark));
        if rail {
            self.with_tooltip(entry.into(), name)
        } else {
            entry.into()
        }
    }

    /// `content` with `name` beside it on the right while hovered.
    fn with_tooltip<'a>(
        &self,
        content: Element<'a, Message>,
        name: String,
    ) -> Element<'a, Message> {
        tooltip(
            content,
            container(label(name, TEXT_CAPTION))
                .padding([6, 10])
                .style(theme::boxed),
            tooltip::Position::Right,
        )
        .gap(6)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_grows_with_the_window_between_its_limits_and_is_a_rail_below_them() {
        assert_eq!(panel_width(700.0, false), RAIL_W);
        assert_eq!(panel_width(959.0, false), RAIL_W);
        assert_eq!(panel_width(960.0, false), PANEL_MIN_W);
        assert_eq!(panel_width(1418.0, false), 269.0);
        assert_eq!(panel_width(2560.0, false), PANEL_MAX_W);
        // Collapsed, the panel is the rail at any width.
        assert_eq!(panel_width(1418.0, true), RAIL_W);
        assert_eq!(panel_width(2560.0, true), RAIL_W);
    }
}
