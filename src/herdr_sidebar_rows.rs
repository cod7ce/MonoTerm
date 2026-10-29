// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Fabien Penso
//
// Adapted from Herdr UI, revision 38ea4248d2c64879f02053b419bdd5386eebce10:
// crates/herdr-gpui/src/sidebar/{row.rs,cell.rs,metrics.rs,agents.rs,layout.rs,
// layouts/herdr.rs}. The fixed-width row/segment layout and label/status policy
// are retained; MonoTerm supplies its shared font, palette, and daemon snapshot.
// GPUI text shaping replaces unicode-width without adding a dependency.

use super::SIDEBAR_WIDTH;
use crate::{appearance::Palette, locale::Language, ui};
use gpui::{
    Div, Font, FontWeight, SharedString, Stateful, TextRun, WindowTextSystem, div, prelude::*, px,
    rgb, rgba,
};
use herdr_client::text::safe_text;
use herdr_protocol::{AgentStatus, ClientShellAgent, ClientShellSnapshot, ClientShellWorkspace};
use std::sync::Arc;

// Herdr's normal/flat density keeps the branch line with no padding between rows.
pub(super) const ROW_PADDING: f32 = 8.0;
const LABEL_GAP: f32 = 6.0;
const STATUS_WIDTH: f32 = 8.0;
const STATUS_DOT_UNKNOWN: f32 = 3.0;
const CAPTION_LIMIT: usize = 160;

pub(super) struct RowContext<'a> {
    pub(super) font: &'a Font,
    pub(super) font_size: f32,
    pub(super) palette: Palette,
    pub(super) language: Language,
    pub(super) text_system: Arc<WindowTextSystem>,
}

impl RowContext<'_> {
    pub(super) fn line_height(&self) -> f32 {
        self.font_size * 4.0 / 3.0
    }

    pub(super) fn header_height(&self) -> f32 {
        self.line_height() + 8.0
    }

    pub(super) fn footer_height(&self) -> f32 {
        self.line_height() + 6.0
    }

    fn glyph_width(&self) -> f32 {
        self.font_size * 0.62
    }

    fn text_width(&self, text: &str, weight: FontWeight) -> f32 {
        let mut font = self.font.clone();
        font.weight = weight;
        let run = TextRun {
            len: text.len(),
            font,
            color: rgb(self.palette.text).into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(
            self.text_system
                .shape_line(
                    SharedString::from(text.to_owned()),
                    px(self.font_size),
                    &[run],
                    None,
                )
                .width,
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Workspace,
    Agent,
}

pub(super) struct Row {
    key: String,
    name: Vec<(String, bool)>,
    detail: String,
    kind: RowKind,
    status: AgentStatus,
    state_label: Option<String>,
    ahead_behind: Option<(usize, usize)>,
}

impl Row {
    pub(super) fn workspace(workspace: &ClientShellWorkspace) -> Self {
        Self {
            key: format!("herdr-space-{}", workspace.workspace_id),
            name: vec![(caption(&workspace.label), true)],
            detail: workspace.branch.as_deref().map(caption).unwrap_or_default(),
            kind: RowKind::Workspace,
            status: workspace.agent_status,
            state_label: None,
            ahead_behind: workspace.git_ahead_behind.filter(|(a, b)| *a > 0 || *b > 0),
        }
    }

    pub(super) fn agent(agent: &ClientShellAgent, snapshot: &ClientShellSnapshot) -> Self {
        // Herdr's agent_names/first_text policy names the integration below its
        // location, with a renamed pane or title as another location segment.
        let name = [
            agent.display_agent.as_deref(),
            agent.name.as_deref(),
            agent.agent.as_deref(),
            agent.title.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(caption)
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| "agent".into());
        let pane = snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == agent.pane_id)
            .and_then(|pane| pane.label.as_deref())
            .map(caption)
            .filter(|name| !name.is_empty())
            .or_else(|| agent.title.as_deref().map(caption))
            .filter(|pane| !pane.is_empty() && pane != &name);
        let workspace = snapshot
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == agent.workspace_id);
        let (mut segments, detail) = match workspace {
            Some(workspace) => {
                let mut segments = vec![(caption(&workspace.label), true)];
                let tab_count = snapshot
                    .tabs
                    .iter()
                    .filter(|tab| tab.workspace_id == agent.workspace_id)
                    .count();
                if let Some(tab) = snapshot.tabs.iter().find(|tab| {
                    tab.tab_id == agent.tab_id && tab.workspace_id == agent.workspace_id
                }) && (tab_count > 1 || tab.custom_label)
                {
                    segments.push((caption(&tab.label), false));
                }
                (segments, name)
            }
            None => (vec![(name, true)], String::new()),
        };
        if let Some(pane) = pane {
            segments.push((pane, false));
        }
        segments.retain(|(text, _)| !text.is_empty());
        Self {
            key: format!("herdr-agent-{}", agent.pane_id),
            name: segments,
            detail,
            kind: RowKind::Agent,
            status: agent.agent_status,
            state_label: agent
                .state_labels
                .iter()
                .find(|(state, _)| state == status_key(agent.agent_status))
                .map(|(_, label)| caption(label))
                .filter(|label| !label.is_empty()),
            ahead_behind: None,
        }
    }

    pub(super) fn height(&self, cx: &RowContext<'_>) -> f32 {
        cx.line_height()
            * if self.kind == RowKind::Workspace
                || !self.detail.is_empty()
                || self.ahead_behind.is_some()
            {
                2.0
            } else {
                1.0
            }
    }

    pub(super) fn render(
        self,
        selected: bool,
        enabled: bool,
        cx: &RowContext<'_>,
    ) -> Stateful<Div> {
        let palette = cx.palette;
        let height = self.height(cx);
        let label_width = SIDEBAR_WIDTH - 1.0 - 2.0 * ROW_PADDING - STATUS_WIDTH - LABEL_GAP;
        let weight = if selected || self.kind == RowKind::Agent {
            FontWeight::BOLD
        } else {
            cx.font.weight
        };
        let muted = if enabled {
            palette.muted
        } else {
            palette.subtle
        };
        let name_color = if !enabled {
            palette.subtle
        } else if selected {
            palette.text
        } else {
            mix(palette.shell, palette.text, 78)
        };
        let detail_color = if enabled && selected && self.kind == RowKind::Workspace {
            palette.accent
        } else {
            muted
        };
        let status_label = self
            .state_label
            .unwrap_or_else(|| cx.language.text(status_label(self.status)).into());
        let mut column = div()
            .w(px(label_width))
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(name_line(
                &self.name,
                (name_color, weight, muted),
                label_width,
                cx,
            ));
        if height > cx.line_height() {
            column = column.child(detail_line(
                &self.detail,
                self.ahead_behind,
                detail_color,
                label_width,
                cx,
            ));
        }
        div()
            .id(SharedString::from(self.key))
            .h(px(height))
            .w_full()
            .min_w_0()
            .flex_none()
            .px(px(ROW_PADDING))
            .flex()
            .items_start()
            .gap(px(LABEL_GAP))
            .when(selected, |row| {
                row.bg(rgba(palette.control_selected(false)))
            })
            .when(enabled, |row| {
                ui::choice_navigation(ui::keyboard_control(row, palette))
                    .cursor_pointer()
                    .hover(move |style| {
                        style.bg(rgba(if selected {
                            palette.control_selected(true)
                        } else {
                            palette.control_hover()
                        }))
                    })
                    .active(move |style| style.bg(rgba(palette.control_pressed())))
            })
            .child(status_dot(self.status, status_label, cx))
            .child(column)
    }
}

/// Fixed columns preserve GPUI 0.2.2 ellipsis measurement. Each segment keeps
/// some room before longer names grow, as Herdr's name_line/segment_budgets do.
fn name_line(
    segments: &[(String, bool)],
    appearance: (u32, FontWeight, u32),
    width: f32,
    cx: &RowContext<'_>,
) -> Div {
    let (primary, weight, muted) = appearance;
    let glyph = cx.glyph_width();
    let separators = segments.len().saturating_sub(1);
    let available = (width - separators as f32 * 3.0 * glyph).max(0.0);
    let lengths: Vec<_> = segments
        .iter()
        .map(|(text, primary)| {
            let weight = if *primary { weight } else { cx.font.weight };
            (cx.text_width(text, weight) / glyph).ceil() as usize
        })
        .collect();
    let budgets = segment_budgets(&lengths, (available / glyph).floor() as usize);
    let used = budgets.iter().sum::<usize>() as f32 * glyph;
    let slack = (available - used).max(0.0);
    let parts =
        segments
            .iter()
            .zip(budgets)
            .enumerate()
            .map(|(index, ((text, is_primary), budget))| {
                let advance = budget as f32 * glyph
                    + if index + 1 == segments.len() {
                        slack
                    } else {
                        0.0
                    };
                let cell = div()
                    .truncate()
                    .when(*is_primary, |part| {
                        part.font_weight(weight).text_color(rgb(primary))
                    })
                    .when(!*is_primary, |part| part.text_color(rgb(muted)))
                    .child(text.clone());
                (if index == 0 { "" } else { " · " }, advance, cell)
            });
    place_line(parts, width, muted, cx)
}

fn place_line(
    parts: impl IntoIterator<Item = (&'static str, f32, Div)>,
    width: f32,
    muted: u32,
    cx: &RowContext<'_>,
) -> Div {
    let mut line = div()
        .relative()
        .w(px(width))
        .h(px(cx.line_height()))
        .flex_none()
        .overflow_hidden();
    let mut x = 0.0;
    for (separator, advance, cell) in parts {
        if !separator.is_empty() {
            let gap = separator.chars().count() as f32 * cx.glyph_width();
            line = line.child(
                div()
                    .absolute()
                    .left(px(x))
                    .w(px(gap))
                    .text_color(rgb(muted))
                    .child(separator),
            );
            x += gap;
        }
        line = line.child(cell.absolute().left(px(x)).w(px(advance)));
        x += advance;
    }
    line
}

fn segment_budgets(lengths: &[usize], available: usize) -> Vec<usize> {
    let mut budgets: Vec<usize> = lengths
        .iter()
        .map(|length| usize::from(*length > 0))
        .collect();
    let mut remaining = available.saturating_sub(budgets.iter().sum());
    while remaining > 0 {
        let mut grew = false;
        for (budget, length) in budgets.iter_mut().zip(lengths) {
            if *budget > 0 && *budget < *length {
                *budget += 1;
                remaining -= 1;
                grew = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !grew {
            break;
        }
    }
    budgets
}

fn detail_line(
    detail: &str,
    ahead_behind: Option<(usize, usize)>,
    color: u32,
    width: f32,
    cx: &RowContext<'_>,
) -> Div {
    let glyph = cx.glyph_width();
    let mut counts = Vec::new();
    if let Some((ahead, behind)) = ahead_behind {
        if ahead > 0 {
            counts.push((
                format!("↑{ahead}"),
                status_color(AgentStatus::Idle, cx.palette),
            ));
        }
        if behind > 0 {
            counts.push((
                format!("↓{behind}"),
                status_color(AgentStatus::Blocked, cx.palette),
            ));
        }
    }
    let counts_width = counts
        .iter()
        .map(|(text, _)| cx.text_width(text, cx.font.weight))
        .sum::<f32>()
        + counts.len().saturating_sub(1) as f32 * glyph;
    let detail_width = if counts.is_empty() {
        width
    } else {
        cx.text_width(detail, cx.font.weight)
            .min((width - counts_width - glyph).max(0.0))
    };
    let counts_x = if detail.is_empty() {
        0.0
    } else {
        detail_width + glyph
    };
    div()
        .relative()
        .w(px(width))
        .h(px(cx.line_height()))
        .flex_none()
        .overflow_hidden()
        .child(
            div()
                .w(px(detail_width))
                .truncate()
                .text_color(rgb(color))
                .child(detail.to_owned()),
        )
        .when(!counts.is_empty(), |line| {
            line.child(
                div()
                    .absolute()
                    .left(px(counts_x))
                    .top_0()
                    .w(px((width - counts_x).max(0.0)))
                    .overflow_hidden()
                    .flex()
                    .gap(px(glyph))
                    .children(
                        counts.into_iter().map(|(text, color)| {
                            div().flex_none().text_color(rgb(color)).child(text)
                        }),
                    ),
            )
        })
}

fn status_dot(status: AgentStatus, label: String, cx: &RowContext<'_>) -> Stateful<Div> {
    let diameter = if status == AgentStatus::Unknown {
        STATUS_DOT_UNKNOWN
    } else {
        STATUS_WIDTH
    };
    let color = status_color(status, cx.palette);
    div()
        .id("status")
        .w(px(STATUS_WIDTH))
        .h(px(STATUS_WIDTH))
        .mt(px((cx.line_height() - STATUS_WIDTH) / 2.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .tooltip(move |_, cx| cx.new(|_| StatusTooltip(label.clone())).into())
        .child(
            div()
                .size(px(diameter))
                .rounded_full()
                .border_1()
                .border_color(rgb(color))
                .when(status != AgentStatus::Idle, |dot| dot.bg(rgb(color))),
        )
}

pub(in super::super) fn status_color(status: AgentStatus, palette: Palette) -> u32 {
    // Herdr's status_style uses these Mocha colors independently of terminal
    // ANSI slots. The light variant darkens them for MonoTerm's light chrome.
    let color = match status {
        AgentStatus::Working => 0xf9e2af,
        AgentStatus::Blocked => 0xf38ba8,
        AgentStatus::Done => 0x94e2d5,
        AgentStatus::Idle => 0xa6e3a1,
        AgentStatus::Unknown => 0x6c7086,
    };
    if palette.dark {
        color
    } else {
        mix(0, color, 45)
    }
}

fn status_key(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Working => "working",
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Idle => "idle",
        AgentStatus::Unknown => "unknown",
    }
}

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "Idle",
        AgentStatus::Working => "Working",
        AgentStatus::Blocked => "Needs attention",
        AgentStatus::Done => "Done",
        AgentStatus::Unknown => "Unknown",
    }
}

fn caption(text: &str) -> String {
    safe_text(text, CAPTION_LIMIT).trim().to_owned()
}

fn mix(base: u32, over: u32, percent: u32) -> u32 {
    let channel = |shift: u32| {
        (((base >> shift) & 255u32) * (100 - percent) + ((over >> shift) & 255u32) * percent) / 100
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

struct StatusTooltip(String);

impl gpui::Render for StatusTooltip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().child(self.0.clone())
    }
}

pub(super) fn agents(snapshot: &ClientShellSnapshot) -> Vec<&ClientShellAgent> {
    // A named plugin view owns both filtering and order, including an empty list.
    if snapshot.agent_view_label.is_some() {
        snapshot
            .agent_order
            .iter()
            .filter_map(|id| snapshot.agents.iter().find(|agent| agent.pane_id == *id))
            .collect()
    } else {
        snapshot.agents.iter().collect()
    }
}
