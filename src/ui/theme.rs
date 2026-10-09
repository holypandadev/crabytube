//! Digital-brutalist design tokens and widget styles.
//!
//! Pure black, white 1 px outlines, square corners, one monospace family and
//! no gradients, blur or shadows. Every view builds from these helpers so the
//! look stays consistent and cheap to rasterize on the CPU renderer.

use super::Message;
use iced::widget::{button, container, pick_list, scrollable, slider, svg, text, text_input};
use iced::{Background, Border, Color, Element, Font, Length, Theme};

pub(super) const BLACK: Color = Color::from_rgb8(0, 0, 0);
pub(super) const INK: Color = Color::from_rgb8(255, 255, 255);
/// Secondary text: metadata, captions and table headings.
pub(super) const DIM: Color = Color::from_rgb8(160, 160, 160);
/// Tertiary text and disabled controls.
pub(super) const FADED: Color = Color::from_rgb8(96, 96, 96);
/// Thin separators between table rows.
pub(super) const RULE: Color = Color::from_rgb8(58, 58, 58);
/// Hover fill for rows and inactive tabs.
pub(super) const HOVER: Color = Color::from_rgb8(26, 26, 26);
pub(super) const ALERT: Color = Color::from_rgb8(255, 92, 92);

pub(super) const FONT_FAMILY: &str = "JetBrains Mono";
pub(super) const MONO: Font = Font::with_name(FONT_FAMILY);
pub(super) const MONO_BOLD: Font = Font {
    weight: iced::font::Weight::Bold,
    ..Font::with_name(FONT_FAMILY)
};

pub(super) const FONT_REGULAR_BYTES: &[u8] =
    include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
pub(super) const FONT_BOLD_BYTES: &[u8] =
    include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf");

/// Page gutters and the vertical rhythm shared by every page.
pub(super) const GUTTER: f32 = 32.0;
pub(super) const TITLE_BAR_H: f32 = 34.0;
pub(super) const NAV_H: f32 = 76.0;
pub(super) const PLAYER_H: f32 = 84.0;

/// Outline width in layout units: one physical pixel at the current interface
/// zoom, so outlines stay hairline-thin when the whole interface is scaled up.
static HAIRLINE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

pub(super) fn hairline() -> f32 {
    f32::from_bits(HAIRLINE.load(std::sync::atomic::Ordering::Relaxed))
}

pub(super) fn set_hairline_for_zoom(zoom: f32) {
    let width = 1.0 / zoom.max(1.0);
    HAIRLINE.store(width.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

pub(super) fn outline(width: f32) -> Border {
    Border {
        color: INK,
        width,
        radius: 0.0.into(),
    }
}

pub(super) fn boxed(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BLACK)),
        border: outline(hairline()),
        ..Default::default()
    }
}

pub(super) fn frame(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BLACK)),
        text_color: Some(INK),
        border: outline(hairline()),
        ..Default::default()
    }
}

pub(super) fn filled(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
        ..Default::default()
    }
}

/// A hairline horizontal rule.
pub(super) fn hrule<'a>(color: Color) -> Element<'a, Message> {
    container(iced::widget::Space::new())
        .width(Length::Fill)
        .height(hairline())
        .style(filled(color))
        .into()
}

/// A hairline vertical rule of the given height.
pub(super) fn vrule<'a>(height: f32, color: Color) -> Element<'a, Message> {
    container(iced::widget::Space::new())
        .width(hairline())
        .height(height)
        .style(filled(color))
        .into()
}

pub(super) fn label<'a>(
    content: impl iced::widget::text::IntoFragment<'a>,
    size: f32,
) -> iced::widget::Text<'a> {
    text(content).size(size).font(MONO)
}

/// Outlined rectangular button that inverts on hover and press.
pub(super) fn outline_button(_: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Hovered | button::Status::Pressed => (INK, BLACK),
        button::Status::Disabled => (BLACK, FADED),
        button::Status::Active => (BLACK, INK),
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border {
            color: if status == button::Status::Disabled {
                FADED
            } else {
                INK
            },
            width: hairline(),
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// Segmented tab: solid white when selected, plain text otherwise.
pub(super) fn tab_style(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let (background, text_color) = if active {
            (INK, BLACK)
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            (HOVER, INK)
        } else {
            (BLACK, INK)
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: Border::default(),
            ..Default::default()
        }
    }
}

/// Borderless icon button; hovering draws the 1 px outline.
pub(super) fn icon_button(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: INK,
        border: Border {
            color: if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                INK
            } else {
                Color::TRANSPARENT
            },
            width: hairline(),
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// Icon button whose outline stays visible, for an opened menu or a toggle.
pub(super) fn icon_button_active(_: &Theme, _: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: INK,
        border: outline(hairline()),
        ..Default::default()
    }
}

/// Invisible button for whole rows and cards; hovering fills the row.
pub(super) fn row_button(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(
            if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                HOVER
            } else {
                Color::TRANSPARENT
            },
        )),
        text_color: INK,
        border: Border::default(),
        ..Default::default()
    }
}

/// The playing row in a list: a permanent hover fill.
pub(super) fn row_button_current(_: &Theme, _: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(HOVER)),
        text_color: INK,
        border: Border::default(),
        ..Default::default()
    }
}

/// Selected row or suggestion: inverted colours.
pub(super) fn inverted_button(_: &Theme, _: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(INK)),
        text_color: BLACK,
        border: Border::default(),
        ..Default::default()
    }
}

pub(super) fn bare_button(_: &Theme, _: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: INK,
        border: Border::default(),
        ..Default::default()
    }
}

pub(super) fn input_style(_: &Theme, _: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border::default(),
        icon: INK,
        placeholder: DIM,
        value: INK,
        selection: Color::from_rgb8(70, 70, 70),
    }
}

pub(super) fn pick_style(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    pick_list::Style {
        text_color: INK,
        placeholder_color: DIM,
        handle_color: INK,
        background: Background::Color(if matches!(status, pick_list::Status::Hovered) {
            HOVER
        } else {
            BLACK
        }),
        border: outline(hairline()),
    }
}

pub(super) fn menu_style(_: &Theme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: Background::Color(BLACK),
        border: outline(hairline()),
        text_color: INK,
        selected_text_color: BLACK,
        selected_background: Background::Color(INK),
        shadow: Default::default(),
    }
}

/// Height every slider is laid out with; the handle is this tall when shown.
pub(super) const SLIDER_H: f32 = 14.0;

/// Modern flat rail: the played part is white, the rest a dark track. The rail
/// thickens and a square handle appears only on hover or while dragging.
pub(super) fn seek_style(_: &Theme, status: slider::Status) -> slider::Style {
    let active = matches!(status, slider::Status::Hovered | slider::Status::Dragged);
    slider::Style {
        rail: slider::Rail {
            backgrounds: (
                Background::Color(INK),
                Background::Color(Color::from_rgb8(52, 52, 52)),
            ),
            width: if active { 5.0 } else { 3.0 },
            border: Border::default(),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Rectangle {
                width: if active { 12 } else { 0 },
                border_radius: 0.0.into(),
            },
            background: Background::Color(INK),
            border_width: if active { 2.0 } else { 0.0 },
            border_color: BLACK,
        },
    }
}

/// Vertical scrolling without a visible scrollbar or a reserved gutter.
pub(super) fn vbar() -> scrollable::Direction {
    scrollable::Direction::Vertical(scrollable::Scrollbar::hidden())
}

pub(super) fn scroll_style(theme: &Theme, status: scrollable::Status) -> scrollable::Style {
    let mut style = scrollable::default(theme, status);
    for rail in [&mut style.vertical_rail, &mut style.horizontal_rail] {
        rail.background = None;
        rail.border = Border::default();
        rail.scroller.background = Background::Color(FADED);
        rail.scroller.border = Border::default();
    }
    style
}

/// Monochrome line icon from the bundled SVG set.
pub(super) fn icon<'a>(name: &'static str, size: f32, color: Color) -> Element<'a, Message> {
    svg(icon_handle(name))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

fn icon_handle(name: &'static str) -> svg::Handle {
    use std::sync::LazyLock;
    macro_rules! icons {
        ($($name:literal),* $(,)?) => {{
            static HANDLES: LazyLock<Vec<(&'static str, svg::Handle)>> = LazyLock::new(|| {
                vec![$(($name, svg::Handle::from_memory(include_bytes!(concat!("../../assets/icons/", $name, ".svg")).as_slice())),)*]
            });
            HANDLES
                .iter()
                .find(|(key, _)| *key == name)
                .or_else(|| HANDLES.first())
                .map(|(_, handle)| handle.clone())
                .expect("bundled icon set is not empty")
        }};
    }
    icons!(
        "search",
        "play",
        "pause",
        "next",
        "prev",
        "shuffle",
        "repeat",
        "repeat-one",
        "queue",
        "volume",
        "volume-off",
        "like",
        "like-filled",
        "close",
        "chev-left",
        "chev-right",
        "more",
        "plus",
        "minus",
        "square",
    )
}

/// A brutalist on/off switch: a white block with a black stop when on, an
/// outlined box with a white stop when off.
pub(super) fn toggle<'a>(on: bool, message: Message) -> Element<'a, Message> {
    let knob = container(iced::widget::Space::new())
        .width(14)
        .height(14)
        .style(filled(if on { BLACK } else { INK }));
    let track: Element<'a, Message> = if on {
        iced::widget::row![iced::widget::Space::new().width(Length::Fill), knob].into()
    } else {
        iced::widget::row![knob, iced::widget::Space::new().width(Length::Fill)].into()
    };
    button(
        container(track)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_y(Length::Fill),
    )
    .on_press(message)
    .width(48)
    .height(24)
    .padding([0, 4])
    .style(move |_, status| button::Style {
        background: Some(Background::Color(if on {
            INK
        } else if matches!(status, button::Status::Hovered) {
            HOVER
        } else {
            BLACK
        })),
        text_color: INK,
        border: outline(hairline()),
        ..Default::default()
    })
    .into()
}

pub(super) fn app_theme() -> Theme {
    use std::sync::LazyLock;
    static THEME: LazyLock<Theme> = LazyLock::new(|| {
        Theme::custom(
            "CrabyTube Brutalist",
            iced::theme::Palette {
                background: BLACK,
                text: INK,
                primary: INK,
                success: INK,
                warning: INK,
                danger: ALERT,
            },
        )
    });
    THEME.clone()
}
