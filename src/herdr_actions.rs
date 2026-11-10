use super::HerdrWindow;
use crate::{herdr_connection::HerdrConnection, window_preferences::HerdrPreferences};
use gpui::Context;
use herdr_protocol::ClientShellSnapshot;

fn cycle_target<'a>(ids: &[&'a str], current: Option<&str>, forward: bool) -> Option<&'a str> {
    if ids.is_empty() {
        return None;
    }
    let index = current.and_then(|id| ids.iter().position(|candidate| *candidate == id));
    let next = match index {
        Some(index) if forward => (index + 1) % ids.len(),
        Some(index) => (index + ids.len() - 1) % ids.len(),
        None if forward => 0,
        None => ids.len() - 1,
    };
    ids.get(next).copied()
}

fn tab_ids(snapshot: &ClientShellSnapshot) -> Vec<&str> {
    snapshot
        .tabs
        .iter()
        .filter(|tab| Some(&tab.workspace_id) == snapshot.focused_workspace_id.as_ref())
        .map(|tab| tab.tab_id.as_str())
        .collect()
}

impl HerdrWindow {
    pub(super) fn save_layout(&mut self, cx: &mut Context<Self>) {
        let preferences = HerdrPreferences {
            visible: self.sidebar_visible,
            width: self.sidebar_width,
            ratio: self.sidebar_ratio,
            view: self.sidebar_view,
            collapsed_repos: self.collapsed_repos.clone(),
        };
        let owner = self.owner.clone();
        let view = cx.weak_entity();
        // Updating another native window while this one renders can reenter its entity.
        cx.defer(move |cx| {
            let result = owner.update(cx, |root, cx| {
                root.window_preferences.herdr = preferences;
                let result = root.window_preferences.save();
                if let Err(error) = &result {
                    root.appearance_notice = Some(error.clone());
                }
                cx.notify();
                result
            });
            if let Err(error) = result {
                eprintln!("{error}");
                if let Err(error) = view.update(cx, |view, cx| view.record(Err(error), cx)) {
                    eprintln!("Herdr layout notice: {error}");
                }
            }
        });
    }

    pub(super) fn actions_available(&self, cx: &Context<Self>) -> bool {
        self.workspace_menu.is_none()
            && self.sidebar_menu.is_none()
            && self.tab_menu.is_none()
            && self.pane_edit.is_none()
            && !self.link_menu_open()
            && self.composition.is_empty()
            && self
                .connection
                .read(cx)
                .surface
                .as_ref()
                .is_none_or(|surface| surface.popup.is_none())
    }

    fn perform(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut HerdrConnection) -> Result<(), String>,
    ) {
        if !self.actions_available(cx) {
            return;
        }
        self.split_drag = None;
        self.cancel_links();
        let result = self.connection.update(cx, |connection, cx| {
            let result = action(connection);
            cx.notify();
            result
        });
        self.record(result, cx);
    }

    pub(super) fn new_tab(&mut self, cx: &mut Context<Self>) {
        self.perform(cx, HerdrConnection::create_tab);
    }

    pub(super) fn close_tab(&mut self, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.focused_tab_id.clone());
        if let Some(id) = id {
            self.perform(cx, |connection| connection.close_tab(&id));
        }
    }

    pub(super) fn close_pane(&mut self, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone());
        if let Some(id) = id {
            self.perform(cx, |connection| connection.close_pane(&id));
        }
    }

    pub(super) fn clear_pane(&mut self, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone());
        if let Some(id) = id {
            self.perform(cx, |connection| connection.clear_pane(&id));
        }
    }

    pub(super) fn split_pane(&mut self, direction: &str, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone());
        if let Some(id) = id {
            self.perform(cx, |connection| connection.split_pane(&id, direction));
        }
    }

    pub(super) fn toggle_pane_zoom(&mut self, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.focused_pane_id.clone());
        if let Some(id) = id {
            self.perform(cx, |connection| connection.toggle_pane_zoom(&id));
        }
    }

    pub(super) fn select_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| tab_ids(snapshot).get(index).map(|id| (*id).to_owned()));
        if let Some(id) = id {
            self.perform(cx, |connection| connection.focus_tab(&id));
        }
    }

    pub(super) fn cycle_tab(&mut self, forward: bool, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                cycle_target(
                    &tab_ids(snapshot),
                    snapshot.focused_tab_id.as_deref(),
                    forward,
                )
                .map(str::to_owned)
            });
        if let Some(id) = id {
            self.perform(cx, |connection| connection.focus_tab(&id));
        }
    }

    pub(super) fn cycle_pane(&mut self, forward: bool, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                let ids: Vec<_> = snapshot
                    .panes
                    .iter()
                    .filter(|pane| Some(&pane.tab_id) == snapshot.focused_tab_id.as_ref())
                    .map(|pane| pane.pane_id.as_str())
                    .collect();
                cycle_target(&ids, snapshot.focused_pane_id.as_deref(), forward).map(str::to_owned)
            });
        if let Some(id) = id {
            self.perform(cx, |connection| connection.focus_pane(&id));
        }
    }

    pub(super) fn cycle_agent(&mut self, forward: bool, cx: &mut Context<Self>) {
        let id = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                let ids: Vec<_> = super::sidebar::rows::agents(snapshot)
                    .iter()
                    .map(|agent| agent.pane_id.as_str())
                    .collect();
                cycle_target(&ids, snapshot.focused_pane_id.as_deref(), forward).map(str::to_owned)
            });
        if let Some(id) = id {
            self.perform(cx, |connection| connection.focus_pane(&id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_wraps_in_display_order_and_handles_missing_focus() {
        let ids = ["third", "first", "second"];
        assert_eq!(cycle_target(&ids, Some("third"), false), Some("second"));
        assert_eq!(cycle_target(&ids, Some("second"), true), Some("third"));
        assert_eq!(cycle_target(&ids, Some("third"), true), Some("first"));
        assert_eq!(cycle_target(&ids, Some("outside"), true), Some("third"));
        assert_eq!(cycle_target(&ids, None, false), Some("second"));
        assert_eq!(cycle_target(&[], None, true), None);
    }

    #[test]
    fn tab_numbers_are_literal_indices_within_current_space() {
        let mut snapshot: ClientShellSnapshot = serde_json::from_str(include_str!(
            "../vendor/herdr/herdr-protocol/tests/fixtures/endpoint-snapshot-v1.json"
        ))
        .unwrap();
        let mut other = snapshot.tabs[0].clone();
        other.workspace_id = "other-space".into();
        other.tab_id = "other-tab".into();
        snapshot.tabs.insert(0, other);
        let tabs = tab_ids(&snapshot);
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0], snapshot.focused_tab_id.as_deref().unwrap());
        assert!(tabs.get(8).is_none());
    }
}
