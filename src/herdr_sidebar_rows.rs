// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Fabien Penso
//
// Agent ordering and labels adapted from Herdr UI at
// 38ea4248d2c64879f02053b419bdd5386eebce10. Rows share MonoTerm's shell UI.
use super::{
    super::{HerdrWindow, indicators},
    workspaces,
};
use crate::{appearance::Palette, ui, window_chrome};
use gpui::{
    Div, FontWeight, HighlightStyle, SharedString, Stateful, StyledText, div, prelude::*, px, rgb,
};
use herdr_client::text::safe_text;
use herdr_protocol::{AgentStatus, ClientShellAgent, ClientShellSnapshot};

pub(super) struct Row {
    key: String,
    label: String,
    context: String,
    detail: String,
    counts: (usize, usize),
    icon: Option<&'static str>,
    status: AgentStatus,
    state_label: Option<String>,
    tree: Option<bool>,
}

impl Row {
    pub(super) fn workspace(entry: &workspaces::Entry<'_>) -> Self {
        let workspace = entry.workspace;
        let detail = if entry.child {
            String::new()
        } else {
            workspace.branch.as_deref().map(caption).unwrap_or_default()
        };
        Self {
            key: format!("herdr-space-{}", workspace.workspace_id),
            label: caption(workspaces::label(workspace, entry.child)),
            context: String::new(),
            detail,
            counts: workspace.git_ahead_behind.unwrap_or_default(),
            icon: None,
            status: entry.status,
            state_label: None,
            tree: entry.child.then_some(entry.last_child),
        }
    }

    pub(super) fn agent(agent: &ClientShellAgent, snapshot: &ClientShellSnapshot) -> Self {
        let name = [
            agent.display_agent.as_deref(),
            agent.name.as_deref(),
            agent.agent.as_deref(),
            agent.title.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(caption)
        .find(|value| !value.is_empty())
        .unwrap_or_else(|| "agent".into());
        let workspace = snapshot
            .workspaces
            .iter()
            .find(|space| space.workspace_id == agent.workspace_id);
        let label = workspace
            .map(|workspace| caption(&workspace.label))
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| name.clone());
        let tabs = snapshot
            .tabs
            .iter()
            .filter(|tab| tab.workspace_id == agent.workspace_id);
        let tab_count = tabs.clone().count();
        let mut segments = Vec::new();
        if let Some(tab) = tabs
            .filter(|tab| tab.tab_id == agent.tab_id)
            .find(|tab| tab_count > 1 || tab.custom_label)
        {
            let tab = caption(&tab.label);
            if !tab.is_empty() && tab != label {
                segments.push(tab);
            }
        }
        let pane_label = snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == agent.pane_id)
            .and_then(|pane| pane.label.as_deref())
            .map(caption)
            .filter(|label| !label.is_empty())
            .or_else(|| agent.title.as_deref().map(caption))
            .filter(|label| !label.is_empty() && label != &name);
        if let Some(pane) = pane_label
            && pane != label
            && !segments.contains(&pane)
        {
            segments.push(pane);
        }
        let kind = agent.agent.as_deref().unwrap_or(&name).to_ascii_lowercase();
        let icon = if kind.contains("claude") {
            "agent-claude"
        } else if kind.contains("codex") {
            "agent-codex"
        } else {
            "terminal"
        };
        Self {
            key: format!("herdr-agent-{}", agent.pane_id),
            label,
            context: segments.join(" · "),
            detail: if workspace.is_some() {
                name
            } else {
                String::new()
            },
            counts: (0, 0),
            icon: Some(icon),
            status: agent.agent_status,
            state_label: agent
                .state_labels
                .iter()
                .find(|(state, _)| state == status_key(agent.agent_status))
                .map(|(_, label)| caption(label))
                .filter(|label| !label.is_empty()),
            tree: None,
        }
    }

    pub(super) fn render(self, selected: bool, enabled: bool, view: &HerdrWindow) -> Stateful<Div> {
        let palette = view.palette.herdr_sidebar();
        let font_size = view.font_size;
        let detail_size = (font_size - 1.0).max(10.0);
        let title_height = (font_size * 1.35).ceil();
        let detail_height = if self.detail.is_empty() {
            0.0
        } else {
            (detail_size * 1.35).ceil()
        };
        let status = self
            .state_label
            .unwrap_or_else(|| view.language.text(status_label(self.status)).into());
        let tooltip = [
            self.label.clone(),
            self.context.clone(),
            self.detail.clone(),
            status,
        ]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
        let detail_gap = if self.icon.is_some() && !self.detail.is_empty() {
            5.0
        } else {
            0.0
        };
        let height = title_height + detail_gap + detail_height + 6.0;
        let child = self.tree.is_some();
        let ink = if enabled {
            view.sidebar_style
                .color(&self.label, self.icon.is_some(), palette)
                .unwrap_or(if child {
                    palette.sidebar_child
                } else {
                    palette.text
                })
        } else {
            palette.subtle
        };
        let muted = if enabled {
            palette.muted
        } else {
            palette.subtle
        };
        let detail_ink = if enabled {
            palette.sidebar_detail
        } else {
            palette.subtle
        };
        let context_ink = if self.icon.is_some() {
            detail_ink
        } else {
            muted
        };
        let status_color = if enabled {
            indicators::color(self.status, palette)
        } else {
            palette.subtle
        };
        let counts = || {
            div()
                .flex_none()
                .flex()
                .gap_1()
                .text_size(px(detail_size))
                .when(self.counts.0 > 0, |row| {
                    row.child(
                        div()
                            .text_color(rgb(if enabled {
                                indicators::color(AgentStatus::Idle, palette)
                            } else {
                                muted
                            }))
                            .child(format!("↑{}", self.counts.0)),
                    )
                })
                .when(self.counts.1 > 0, |row| {
                    row.child(
                        div()
                            .text_color(rgb(if enabled {
                                indicators::color(AgentStatus::Blocked, palette)
                            } else {
                                muted
                            }))
                            .child(format!("↓{}", self.counts.1)),
                    )
                })
        };
        let label_length = self.label.len();
        let mut title_text = self.label.clone();
        if !self.context.is_empty() {
            title_text.push_str(" · ");
            title_text.push_str(&self.context);
        }
        let context_range = label_length..title_text.len();
        let title = div()
            .h(px(title_height))
            .line_height(px(title_height))
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .overflow_hidden()
            .when_some(self.icon, |row, icon| {
                row.child(ui::icon(icon).size(px(font_size)).text_color(rgb(muted)))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(font_size))
                    .text_color(rgb(ink))
                    .font_weight(if selected || self.icon.is_some() {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .child(StyledText::new(title_text).with_highlights(
                        (!context_range.is_empty()).then_some((
                            context_range,
                            HighlightStyle {
                                color: Some(rgb(context_ink).into()),
                                font_weight: Some(FontWeight::NORMAL),
                                ..Default::default()
                            },
                        )),
                    )),
            )
            .when(self.detail.is_empty(), |row| row.child(counts()));
        let mut detail_text = self.detail.clone();
        let mut detail_highlights = Vec::new();
        for (count, arrow, status) in [
            (self.counts.0, '↑', AgentStatus::Idle),
            (self.counts.1, '↓', AgentStatus::Blocked),
        ] {
            if count > 0 {
                let start = detail_text.len();
                detail_text.push_str(&format!(" {arrow}{count}"));
                detail_highlights.push((
                    start..detail_text.len(),
                    HighlightStyle {
                        color: Some(
                            rgb(if enabled {
                                indicators::color(status, palette)
                            } else {
                                muted
                            })
                            .into(),
                        ),
                        ..Default::default()
                    },
                ));
            }
        }
        let text = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(title)
            .when(!self.detail.is_empty(), |column| {
                column.child(
                    div()
                        .mt(px(detail_gap))
                        .h(px(detail_height))
                        .line_height(px(detail_height))
                        .flex()
                        .items_center()
                        .gap_1()
                        .when(self.icon.is_some(), |row| row.pl(px(font_size + 4.0)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(detail_size))
                                .text_color(rgb(detail_ink))
                                .child(
                                    StyledText::new(detail_text).with_highlights(detail_highlights),
                                ),
                        ),
                )
            });
        window_chrome::sidebar_row(
            SharedString::from(self.key.clone()),
            selected,
            enabled,
            palette,
        )
        .h(px(height))
        .pl(px(6.0))
        .pr(px(4.0))
        .rounded(px(6.0))
        .gap(px(4.0))
        .relative()
        .when(
            matches!(
                self.status,
                AgentStatus::Working | AgentStatus::Blocked | AgentStatus::Done
            ),
            |row| {
                row.child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(6.0))
                        .bottom(px(6.0))
                        .w(px(2.0))
                        .rounded(px(1.0))
                        .bg(rgb(status_color)),
                )
            },
        )
        .when_some(self.tree, |row, last| {
            row.pl(px(42.0)).child(
                div()
                    .absolute()
                    .left(px(24.0))
                    .top(px(-1.0))
                    .w(px(10.0))
                    .h(px(if last {
                        height / 2.0 + 1.0
                    } else {
                        height + 1.0
                    }))
                    .border_l_1()
                    .border_color(rgb(palette.tree_guide()))
                    .child(
                        div()
                            .absolute()
                            .top(px(height / 2.0 + 1.0))
                            .w_full()
                            .h(px(1.0))
                            .bg(rgb(palette.tree_guide())),
                    ),
            )
        })
        .tooltip(move |_, cx| cx.new(|_| RowTooltip(tooltip.clone(), palette)).into())
        .child(
            div()
                .h_full()
                .pt(px(3.0 + (title_height - 14.0) / 2.0))
                .flex_none()
                .child(indicators::mark(
                    self.status,
                    view.indicator_elapsed,
                    status_color,
                )),
        )
        .child(text)
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
    safe_text(text, 160).trim().to_owned()
}

struct RowTooltip(String, Palette);
impl gpui::Render for RowTooltip {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(px(5.0))
            .bg(rgb(self.1.panel))
            .border_1()
            .border_color(rgb(self.1.border))
            .text_color(rgb(self.1.text))
            .text_size(px(11.0))
            .child(self.0.clone())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_context_omits_default_tabs_and_duplicate_names() {
        let mut snapshot: ClientShellSnapshot = serde_json::from_str(include_str!(
            "../vendor/herdr/herdr-protocol/tests/fixtures/endpoint-snapshot-v1.json"
        ))
        .expect("valid protocol fixture");
        snapshot.workspaces[0].label = "🐸 Repo".into();
        snapshot.tabs[0].label = "1".into();
        snapshot.panes[0].label = Some("Build".into());
        let row = Row::agent(&snapshot.agents[0], &snapshot);
        assert_eq!(row.label, "🐸 Repo");
        assert_eq!(row.context, "Build");
        assert_eq!(row.detail, "Claude");
        snapshot.tabs[0].custom_label = true;
        snapshot.tabs[0].label = "Build".into();
        assert_eq!(Row::agent(&snapshot.agents[0], &snapshot).context, "Build");
        snapshot.panes[0].label = Some(" \n".into());
        assert_eq!(
            Row::agent(&snapshot.agents[0], &snapshot).context,
            "Build · Review"
        );
    }
}
