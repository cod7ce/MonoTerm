use crate::appearance::Palette;
use gpui::{AssetSource, SharedString, div, prelude::*, px, rgb, rgba, svg};
use std::borrow::Cow;

gpui::actions!(controls, [FocusControls]);

pub fn init(cx: &mut gpui::App) {
    cx.bind_keys([gpui::KeyBinding::new("ctrl-f6", FocusControls, None)]);
}

pub const TERMINAL: u32 = 0x141414;
pub const TEXT: u32 = 0xebebeb;
pub const MUTED: u32 = 0xa0a0a0;
pub const SUBTLE: u32 = 0x777777;
pub const GREEN: u32 = 0x8eb39a;
/// Follow-state icon tints for dark and light panels.
pub const FOLLOWING: u32 = 0x66b886;
pub const PINNED: u32 = 0xd7a64f;
pub const FOLLOWING_LIGHT: u32 = 0x287244;
pub const PINNED_LIGHT: u32 = 0x916114;

pub struct Icons;

const ICONS: &[(&str, &str)] = &[
    ("terminal", "<path d='m5 6 5 6-5 6m7 0h7'/>"),
    ("code", "<path d='m8 5-6 7 6 7m8-14 6 7-6 7m-2-16-4 18'/>"),
    (
        "server",
        "<rect x='3' y='3' width='18' height='7' rx='2'/><rect x='3' y='14' width='18' height='7' rx='2'/><path d='M7 6.5h.01M7 17.5h.01'/>",
    ),
    (
        "database",
        "<ellipse cx='12' cy='5' rx='8' ry='3'/><path d='M4 5v14c0 4 16 4 16 0V5M4 12c0 4 16 4 16 0'/>",
    ),
    (
        "globe",
        "<circle cx='12' cy='12' r='9'/><ellipse cx='12' cy='12' rx='4' ry='9'/><path d='M3 12h18'/>",
    ),
    (
        "star",
        "<path d='m12 3 2.8 5.8 6.4.9-4.6 4.5 1.1 6.3L12 17.5l-5.7 3 1.1-6.3-4.6-4.5 6.4-.9Z'/>",
    ),
    ("bolt", "<path d='m13 2-9 12h7l-1 8 10-13h-7Z'/>"),
    (
        "branch",
        "<circle cx='6' cy='5' r='2'/><circle cx='6' cy='19' r='2'/><circle cx='18' cy='5' r='2'/><path d='M6 7v10m0-3c8 0 12-2 12-7'/>",
    ),
    ("plus", "<path d='M12 5v14M5 12h14'/>"),
    ("close", "<path d='m6 6 12 12M18 6 6 18'/>"),
    (
        "sidebar",
        "<rect x='3' y='4' width='18' height='16' rx='2'/><path d='M9 4v16'/>",
    ),
    ("chevron", "<path d='m8 5 7 7-7 7'/>"),
    ("chevron-down", "<path d='m5 8 7 7 7-7'/>"),
    (
        "follow-on",
        "<path d='M10 13a5 5 0 0 0 7 0l2-2a5 5 0 0 0-7-7l-1 1'/><path d='M14 11a5 5 0 0 0-7 0l-2 2a5 5 0 0 0 7 7l1-1'/>",
    ),
    (
        "follow-off",
        "<path d='M10 13a5 5 0 0 0 7 0l2-2a5 5 0 0 0-7-7l-1 1'/><path d='M14 11a5 5 0 0 0-7 0l-2 2a5 5 0 0 0 7 7l1-1'/><path d='m4 4 16 16'/>",
    ),
    (
        "pin",
        "<path d='M9 4h6l-1 5 3 3v2H7v-2l3-3-1-5Z'/><path d='M12 14v7'/>",
    ),
    ("restart", "<path d='M4 10a8 8 0 1 1 1 8M4 4v6h6'/>"),
    (
        "settings",
        "<path d='M5 3v18m7-18v18m7-18v18M2 8h6m1 8h6m1-8h6'/><circle cx='5' cy='8' r='2' fill='black'/><circle cx='12' cy='16' r='2' fill='black'/><circle cx='19' cy='8' r='2' fill='black'/>",
    ),
    (
        "keyboard",
        "<rect x='2' y='5' width='20' height='14' rx='2'/><path d='M5 9h2m3 0h2m3 0h2M5 12h2m3 0h2m3 0h2M6 16h12'/>",
    ),
    (
        "monitor",
        "<rect x='3' y='3' width='18' height='13' rx='2'/><path d='M12 16v5m-5 0h10'/>",
    ),
    ("minus", "<path d='M5 12h14'/>"),
    ("check", "<path d='m5 12 4 4L19 6'/>"),
    (
        "check-thick",
        "<path stroke-width='3' d='m5 12 4.5 4.5L19 7'/>",
    ),
    (
        "split-right",
        "<rect x='3' y='4' width='18' height='16' rx='2'/><path d='M12 4v16'/>",
    ),
    (
        "split-down",
        "<rect x='3' y='4' width='18' height='16' rx='2'/><path d='M3 12h18'/>",
    ),
    ("zen", "<path d='M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5'/>"),
];

const AGENT_ICONS: &[(&str, &[u8])] = &[
    ("agent-claude", include_bytes!("../assets/agent-claude.svg")),
    ("agent-codex", include_bytes!("../assets/agent-codex.svg")),
];

impl AssetSource for Icons {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = AGENT_ICONS.iter().find(|(name, _)| *name == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if path == "app-icon.png" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/AppIcon.png"))));
        }
        if path == "herdr.png" {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/herdr.png"))));
        }
        if path == "herdr-tight.png" {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/herdr-tight.png"
            ))));
        }
        Ok(ICONS.iter().find(|(name, _)| *name == path).map(|(_, body)| {
            Cow::Owned(format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='1.6' stroke-linecap='round' stroke-linejoin='round'>{body}</svg>").into_bytes())
        }))
    }

    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .map(|(name, _)| (*name).into())
            .chain(AGENT_ICONS.iter().map(|(name, _)| (*name).into()))
            .collect())
    }
}

pub fn herdr_icon() -> gpui::Img {
    // The tight crop removes the artwork's transparent margin so the titlebar
    // highlight can hug the visible icon edge.
    gpui::img(gpui::ImageSource::Resource(gpui::Resource::Embedded(
        "herdr-tight.png".into(),
    )))
    .size(px(15.0))
    .flex_shrink_0()
}

pub fn icon(name: &'static str) -> gpui::Svg {
    svg()
        .path(name)
        .size(px(15.0))
        .flex_shrink_0()
        .text_color(rgb(MUTED))
}

pub fn control_icon(
    name: &'static str,
    palette: Palette,
    group: impl Into<SharedString>,
) -> gpui::Svg {
    // This GPUI version paints SVGs only when they have an explicit color.
    icon(name)
        .text_color(rgb(palette.muted))
        .group_hover(group, move |style| style.text_color(rgb(palette.text)))
}

pub fn keyboard_control(
    element: gpui::Stateful<gpui::Div>,
    palette: Palette,
) -> gpui::Stateful<gpui::Div> {
    element
        .tab_index(0)
        .key_context("Control")
        .focus_visible(move |style| style.border_1().border_color(rgb(palette.accent)))
        .on_key_down(|event, window, cx| {
            let stroke = &event.keystroke;
            if stroke.key == "tab"
                && !stroke.modifiers.control
                && !stroke.modifiers.alt
                && !stroke.modifiers.platform
            {
                if stroke.modifiers.shift {
                    window.focus_prev(cx);
                } else {
                    window.focus_next(cx);
                }
                cx.stop_propagation();
            } else if !stroke.modifiers.modified()
                && matches!(stroke.key.as_str(), "enter" | "space")
            {
                // Click activation is handled by GPUI on key-up.
                cx.stop_propagation();
            }
        })
}

pub fn traverse_controls(
    event: &gpui::KeyDownEvent,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) {
    let stroke = &event.keystroke;
    if stroke.key == "tab"
        && !stroke.modifiers.control
        && !stroke.modifiers.alt
        && !stroke.modifiers.platform
    {
        if stroke.modifiers.shift {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        cx.stop_propagation();
    }
}

pub fn choice_navigation(element: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
    element.on_key_down(|event, window, cx| {
        if !event.keystroke.modifiers.modified() {
            match event.keystroke.key.as_str() {
                "left" | "up" => window.focus_prev(cx),
                "right" | "down" => window.focus_next(cx),
                _ => return,
            }
            cx.stop_propagation();
        }
    })
}

pub fn icon_button(
    id: &'static str,
    name: &'static str,
    label: &'static str,
    palette: Palette,
) -> gpui::Stateful<gpui::Div> {
    icon_toggle(id, name, label, palette, false)
}

pub fn icon_toggle(
    id: &'static str,
    name: &'static str,
    label: &'static str,
    palette: Palette,
    selected: bool,
) -> gpui::Stateful<gpui::Div> {
    keyboard_control(div().id(id), palette)
        .group(id)
        .size(px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .text_color(rgb(palette.muted))
        .cursor_pointer()
        .when(selected, |button| {
            button.bg(rgba(palette.control_selected(false)))
        })
        .hover(move |style| {
            style
                .bg(rgba(if selected {
                    palette.control_selected(true)
                } else {
                    palette.control_hover()
                }))
                .text_color(rgb(palette.text))
        })
        .active(move |style| style.bg(rgba(palette.control_pressed())))
        .tooltip(move |_, cx| cx.new(|_| Tooltip(label, palette)).into())
        .child(
            control_icon(name, palette, id)
                .when(selected, |icon| icon.text_color(rgb(palette.accent))),
        )
}

pub fn block_terminal_actions(mut element: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
    let actions: Vec<Box<dyn gpui::Action>> = vec![
        Box::new(crate::NewSession),
        Box::new(crate::CloseTab),
        Box::new(crate::CloseSession),
        Box::new(crate::Restart),
        Box::new(crate::NextSession),
        Box::new(crate::PreviousSession),
        Box::new(crate::NextPane),
        Box::new(crate::PreviousPane),
        Box::new(crate::SplitRight),
        Box::new(crate::SplitDown),
        Box::new(crate::ToggleZen),
        Box::new(crate::ToggleSidebar),
        Box::new(crate::ToggleSettings),
        Box::new(crate::CustomizeTab),
        Box::new(crate::IncreaseFont),
        Box::new(crate::DecreaseFont),
        Box::new(crate::ResetFont),
        Box::new(crate::ResumeFollow),
        Box::new(crate::ReturnLocal),
        Box::new(crate::OpenHerdrWindow),
    ];
    for action in actions {
        element = element.on_boxed_action(action.as_ref(), |_, _, cx| cx.stop_propagation());
    }
    element
}

pub fn dot(running: bool) -> gpui::Div {
    div()
        .size(px(5.0))
        .flex_shrink_0()
        .rounded_full()
        .bg(rgb(if running { GREEN } else { SUBTLE }))
}

pub(crate) struct Tooltip(pub(crate) &'static str, pub(crate) Palette);
impl gpui::Render for Tooltip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let p = self.1;
        div()
            .px_2()
            .py_1()
            .rounded(px(5.0))
            .bg(rgb(p.panel))
            .border_1()
            .border_color(rgb(p.border))
            .text_color(rgb(p.text))
            .text_size(px(11.0))
            .child(self.0)
    }
}
