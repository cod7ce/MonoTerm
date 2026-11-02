//! Shared geometry and sidebar controls for local and Herdr terminal windows.
use crate::{appearance::Palette, ui};
use gpui::{Div, ElementId, Stateful, div, prelude::*, px, rgb, rgba};

pub const SHELL_INSET: f32 = 5.0;
pub const TOOLBAR_CENTER: f32 = 22.0;
pub const TOOLBAR_HEIGHT: f32 = 34.0;
pub const SHELL_RADIUS: f32 = 12.0;
pub const SIDEBAR_WIDTH: f32 = 228.0;
pub const SIDEBAR_HANDLE: f32 = 4.0;
pub const SECTION_HANDLE: f32 = 6.0;
const SECTION_MIN_HEIGHT: f32 = 96.0;

pub fn show_sidebar(visible: bool, window_width: f32) -> bool {
    visible && window_width >= 760.0
}

pub fn sidebar_width(width: f32, window_width: f32, scale: f32) -> f32 {
    let maximum = (window_width - 420.0).clamp(180.0, 480.0);
    (width.clamp(180.0, maximum) * scale).round() / scale
}

pub fn section_height(ratio: f32, available: f32) -> f32 {
    let available = available.max(0.0);
    let minimum = SECTION_MIN_HEIGHT.min(available / 2.0);
    (ratio * available).clamp(minimum, available - minimum)
}

pub fn toolbar() -> Div {
    div()
        .h(px(TOOLBAR_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1()
        .pr_2()
}

pub fn sidebar_toggle_slot(show_sidebar: bool, width: f32) -> Div {
    div()
        .relative()
        .top(px(TOOLBAR_CENTER - (SHELL_INSET + TOOLBAR_HEIGHT / 2.0)))
        .w(px(if show_sidebar {
            width - SIDEBAR_HANDLE
        } else {
            110.0
        }))
        .h_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .pl(px(78.0))
        .pr_1()
        .child(div().flex_1())
}

pub fn sidebar_heading() -> Div {
    div()
        .pl_3()
        .pr_1()
        .h(px(40.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1()
}

pub fn sidebar_row(
    id: impl Into<ElementId>,
    selected: bool,
    enabled: bool,
    p: Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.0))
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .pl(px(10.0))
        .pr_2()
        .flex()
        .items_center()
        .gap_2()
        .rounded(px(6.0))
        .bg(rgba(p.tab_background(selected, false)))
        .when(enabled, |row| {
            ui::choice_navigation(ui::keyboard_control(row, p))
                .cursor_pointer()
                .hover(move |style| style.bg(rgba(p.tab_background(selected, true))))
        })
}

pub fn sidebar_label(selected: bool, enabled: bool, p: Palette) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(px(12.0))
        .font_weight(if selected {
            gpui::FontWeight::MEDIUM
        } else {
            gpui::FontWeight::NORMAL
        })
        .text_color(rgb(if !enabled {
            p.subtle
        } else if selected {
            p.text
        } else {
            p.muted
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_preserves_terminal_space_and_aligns_to_device_pixels() {
        assert!(!show_sidebar(true, 759.0));
        assert!(show_sidebar(true, 760.0));
        assert!(!show_sidebar(false, 1000.0));
        assert_eq!(sidebar_width(480.0, 760.0, 2.0), 340.0);
        assert_eq!(sidebar_width(0.0, 1000.0, 2.0), 180.0);
        assert_eq!(sidebar_width(228.24, 1000.0, 2.0), 228.0);
    }

    #[test]
    fn sidebar_sections_keep_both_lists_reachable_at_drag_limits() {
        assert_eq!(section_height(0.5, 600.0), 300.0);
        assert_eq!(section_height(-1.0, 600.0), 96.0);
        assert_eq!(section_height(2.0, 600.0), 504.0);
        assert_eq!(section_height(0.9, 100.0), 50.0);
        assert_eq!(section_height(0.5, -10.0), 0.0);
    }
}
