//! Workspace dialogs retain the clicked identity and correlate daemon responses.
use super::HerdrWindow;
use crate::{
    herdr_connection::{WorkspaceAction, WorkspaceTarget},
    herdr_worktree::{self, WorktreeList, WorktreeSource},
    text_input::TextInput,
    toast,
};
use gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, Pixels, Point, Window, prelude::*,
};
use herdr_client::Method;
use serde_json::{Value, json};

#[path = "herdr_workspace_menu_render.rs"]
mod render;

pub(super) struct WorkspaceMenu {
    target: WorkspaceTarget,
    position: Point<Pixels>,
    group: Option<String>,
    source: Option<WorktreeSource>,
    focus: FocusHandle,
    items: Vec<FocusHandle>,
    mode: Mode,
    pending: Option<(String, bool)>,
    error: Option<String>,
}

enum Mode {
    Menu,
    Rename(Entity<TextInput>),
    Close(Vec<String>),
    New(Entity<TextInput>),
    Open(Option<WorktreeList>),
    Delete { path: Option<String>, force: bool },
}

#[derive(Clone, Copy)]
enum Action {
    NewWorktree,
    Rename,
    Close,
    OpenWorktree,
    NewTab,
    CopyDirectory,
    CopyBranch,
    Fold,
    DeleteWorktree,
}

impl WorkspaceMenu {
    fn actions(&self) -> Vec<(Action, &'static str, &'static str, bool)> {
        let mut actions = vec![
            (
                Action::NewWorktree,
                "New worktree",
                "branch",
                self.source.is_some(),
            ),
            (Action::Rename, "Rename", "edit", true),
            (
                Action::Close,
                if self.target.close_members.len() > 1 {
                    "Close group"
                } else {
                    "Close Space"
                },
                "close",
                true,
            ),
        ];
        if herdr_worktree::can_open(&self.target) {
            actions.push((Action::OpenWorktree, "Open worktree…", "folder", true));
        }
        actions.push((Action::NewTab, "New tab here", "plus", true));
        actions.push((Action::CopyDirectory, "Copy directory", "folder", true));
        if self.target.workspace.branch.is_some() {
            actions.push((Action::CopyBranch, "Copy branch", "branch", true));
        }
        if self.group.is_some() {
            actions.push((Action::Fold, "Collapse group", "chevron-down", true));
        }
        if herdr_worktree::can_delete(&self.target) {
            actions.push((
                Action::DeleteWorktree,
                "Delete worktree checkout…",
                "trash",
                true,
            ));
        }
        actions
    }

    fn input(&self) -> Option<&Entity<TextInput>> {
        match &self.mode {
            Mode::Rename(input) | Mode::New(input) => Some(input),
            _ => None,
        }
    }
}

impl HerdrWindow {
    pub(super) fn open_workspace_menu(
        &mut self,
        target: WorkspaceTarget,
        group: Option<String>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let source = self
            .connection
            .read(cx)
            .snapshot
            .as_ref()
            .and_then(|snapshot| WorktreeSource::new(snapshot, &target).ok());
        self.sidebar_drag = None;
        self.sidebar_menu = None;
        self.selecting = false;
        self.workspace_menu = Some(WorkspaceMenu {
            target,
            position,
            group,
            source,
            focus,
            items: (0..10)
                .map(|_| cx.focus_handle().tab_index(0).tab_stop(true))
                .collect(),
            mode: Mode::Menu,
            pending: None,
            error: None,
        });
        cx.notify();
    }

    fn dismiss_workspace_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workspace_menu = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn workspace_menu_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = &self.workspace_menu else {
            return;
        };
        let target = menu.target.clone();
        let group = menu.group.clone();
        let source = menu.source.clone();
        let snapshot = self.connection.read(cx).snapshot.clone();
        if let Err(error) = snapshot
            .as_ref()
            .ok_or_else(|| "Herdr has no snapshot".to_owned())
            .and_then(|snapshot| target.validate(snapshot))
        {
            self.dismiss_workspace_menu(window, cx);
            self.record(Err(error), cx);
            return;
        }
        match action {
            Action::Rename | Action::NewWorktree => {
                if matches!(action, Action::NewWorktree) && source.is_none() {
                    return;
                }
                let initial = if matches!(action, Action::Rename) {
                    target.workspace.label.clone()
                } else {
                    String::new()
                };
                let input = cx.new(|cx| TextInput::new(initial, String::new(), cx));
                input.update(cx, |input, cx| input.focus(window, cx));
                if let Some(menu) = &mut self.workspace_menu {
                    menu.mode = if matches!(action, Action::Rename) {
                        Mode::Rename(input)
                    } else {
                        Mode::New(input)
                    };
                }
            }
            Action::Close => {
                let mut panes: Vec<_> = snapshot
                    .iter()
                    .flat_map(|snapshot| &snapshot.panes)
                    .filter(|pane| target.close_members.contains(&pane.workspace_id))
                    .map(|pane| pane.pane_id.clone())
                    .collect();
                panes.sort();
                if let Some(menu) = &mut self.workspace_menu {
                    menu.mode = Mode::Close(panes);
                    menu.items[0].focus(window, cx);
                }
            }
            Action::OpenWorktree | Action::DeleteWorktree => {
                if let Some(menu) = &mut self.workspace_menu {
                    menu.mode = if matches!(action, Action::OpenWorktree) {
                        Mode::Open(None)
                    } else {
                        Mode::Delete {
                            path: None,
                            force: false,
                        }
                    };
                    menu.items[0].focus(window, cx);
                }
                self.queue_worktree(
                    &target,
                    Method::WorktreeList,
                    herdr_worktree::list_params(&target),
                    false,
                    cx,
                );
            }
            Action::NewTab => {
                let result = self.connection.update(cx, |connection, cx| {
                    let result = connection.workspace_action(&target, WorkspaceAction::NewTab);
                    cx.notify();
                    result
                });
                if result.is_ok() {
                    self.dismiss_workspace_menu(window, cx);
                }
                self.record(result, cx);
            }
            Action::CopyDirectory | Action::CopyBranch => {
                let text = match action {
                    Action::CopyDirectory => target.workspace.new_workspace_cwd,
                    _ => target.workspace.branch.unwrap_or_default(),
                };
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                self.dismiss_workspace_menu(window, cx);
                toast::show(self.language.text("Copied"), window, cx);
            }
            Action::Fold => {
                if let Some(key) = group
                    && !self.collapsed_repos.remove(&key)
                {
                    self.collapsed_repos.insert(key);
                }
                self.save_layout(cx);
                self.dismiss_workspace_menu(window, cx);
            }
        }
        cx.notify();
    }

    fn queue_worktree(
        &mut self,
        target: &WorkspaceTarget,
        method: Method,
        params: Value,
        submit: bool,
        cx: &mut Context<Self>,
    ) {
        let result = self.connection.update(cx, |connection, cx| {
            let result = connection.request_worktree(target, method, params);
            cx.notify();
            result
        });
        if let Some(menu) = &mut self.workspace_menu {
            menu.error = result.as_ref().err().cloned();
            menu.pending = result.ok().map(|id| (id, submit));
        }
        cx.notify();
    }

    pub(super) fn update_workspace_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, submit)) = self
            .workspace_menu
            .as_ref()
            .and_then(|menu| menu.pending.clone())
        else {
            return;
        };
        let response = self
            .connection
            .update(cx, |connection, _| connection.take_worktree_response(&id));
        let Some(response) = response else {
            return;
        };
        let snapshot = self.connection.read(cx).snapshot.clone();
        let Some(menu) = &mut self.workspace_menu else {
            return;
        };
        menu.pending = None;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                menu.error = Some(error);
                return;
            }
        };
        if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
            if submit
                && error["code"] == "dirty_worktree_requires_force"
                && let Mode::Delete { force, .. } = &mut menu.mode
            {
                *force = true;
                // A held confirmation key must not activate the newly dangerous action.
                menu.items[0].focus(window, cx);
            }
            menu.error = Some(herdr_client::text::safe_text(
                error["message"]
                    .as_str()
                    .unwrap_or("Herdr rejected the request"),
                800,
            ));
            return;
        }
        if submit {
            if let Some(tree) = &menu.target.workspace.worktree {
                self.collapsed_repos.remove(&tree.key);
            }
            self.save_layout(cx);
            self.dismiss_workspace_menu(window, cx);
            return;
        }
        let target = menu.target.clone();
        let result = snapshot
            .as_ref()
            .ok_or_else(|| "Herdr has no snapshot".to_owned())
            .and_then(|snapshot| target.validate(snapshot))
            .and_then(|()| WorktreeList::parse(&response));
        match result {
            Ok(list) => match &mut menu.mode {
                Mode::Open(entries) => {
                    if target
                        .workspace
                        .worktree
                        .as_ref()
                        .is_some_and(|tree| tree.key != list.repo_key)
                    {
                        menu.error = Some("Herdr repository changed; reopen its menu".into());
                        return;
                    }
                    let len = list.entries.len();
                    if menu.items.len() < len + 2 {
                        menu.items.extend(
                            (menu.items.len()..len + 2)
                                .map(|_| cx.focus_handle().tab_index(0).tab_stop(true)),
                        );
                    }
                    *entries = Some(list);
                }
                Mode::Delete { path, .. } => match list.deletion_path(&target) {
                    Ok(directory) => *path = Some(directory),
                    Err(error) => menu.error = Some(error),
                },
                _ => {}
            },
            Err(error) => menu.error = Some(error),
        }
    }

    fn open_worktree_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(menu) = &self.workspace_menu else {
            return;
        };
        if menu.pending.is_some() {
            return;
        }
        let Mode::Open(Some(list)) = &menu.mode else {
            return;
        };
        let Some(entry) = list.entries.get(index) else {
            return;
        };
        let target = menu.target.clone();
        let params = herdr_worktree::open_params(&target, &entry.path);
        self.queue_worktree(&target, Method::WorktreeOpen, params, true, cx);
    }

    fn confirm_workspace_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = &self.workspace_menu else {
            return;
        };
        if menu.pending.is_some()
            || menu
                .input()
                .is_some_and(|input| input.read(cx).has_marked_text())
        {
            return;
        }
        let target = menu.target.clone();
        let source = menu.source.clone();
        let mode = match &menu.mode {
            Mode::Menu | Mode::Open(_) | Mode::Delete { path: None, .. } => return,
            Mode::Rename(input) => Confirm::Rename(input.read(cx).value().to_owned()),
            Mode::New(input) => Confirm::New(input.read(cx).value().to_owned()),
            Mode::Close(panes) => Confirm::Close(panes.clone()),
            Mode::Delete {
                path: Some(_),
                force,
            } => Confirm::Delete(*force),
        };
        match mode {
            Confirm::New(branch) => {
                let snapshot = self.connection.read(cx).snapshot.clone();
                let result = source
                    .ok_or_else(|| "No main checkout is available".to_owned())
                    .and_then(|source| {
                        source.validate(snapshot.as_ref().ok_or("Herdr has no snapshot")?)?;
                        Ok((source.target.clone(), source.create_params(&branch)?))
                    });
                match result {
                    Ok((source, params)) => {
                        self.queue_worktree(&source, Method::WorktreeCreate, params, true, cx)
                    }
                    Err(error) => {
                        if let Some(menu) = &mut self.workspace_menu {
                            menu.error = Some(error);
                        }
                    }
                }
            }
            Confirm::Delete(force) => {
                let params = json!({"workspace_id": target.workspace.workspace_id, "force": force, "trust_repository": false});
                self.queue_worktree(&target, Method::WorktreeRemove, params, true, cx);
            }
            Confirm::Rename(label) => {
                self.submit_workspace_action(&target, WorkspaceAction::Rename(&label), window, cx)
            }
            Confirm::Close(panes) => {
                self.submit_workspace_action(&target, WorkspaceAction::Close(&panes), window, cx)
            }
        }
        cx.notify();
    }

    fn submit_workspace_action(
        &mut self,
        target: &WorkspaceTarget,
        action: WorkspaceAction<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.connection.update(cx, |connection, cx| {
            let result = connection.workspace_action(target, action);
            cx.notify();
            result
        });
        match result {
            Ok(()) => self.dismiss_workspace_menu(window, cx),
            Err(error) => {
                if let Some(menu) = &mut self.workspace_menu {
                    menu.error = Some(error);
                }
            }
        }
    }

    fn workspace_menu_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = &self.workspace_menu else {
            return;
        };
        if menu
            .input()
            .is_some_and(|input| input.read(cx).has_marked_text())
        {
            return;
        }
        let key = &event.keystroke;
        if key.modifiers.control || key.modifiers.alt || key.modifiers.platform {
            return;
        }
        if event.is_held && matches!(key.key.as_str(), "enter" | "return" | "space") {
            cx.stop_propagation();
            window.prevent_default();
            return;
        }
        if key.key == "escape" {
            self.dismiss_workspace_menu(window, cx);
            cx.stop_propagation();
        } else if key.key == "tab"
            || (menu.input().is_none()
                && matches!(key.key.as_str(), "up" | "down" | "left" | "right"))
        {
            let backwards = matches!(key.key.as_str(), "up" | "left") || key.modifiers.shift;
            let mut handles = Vec::new();
            if let Some(input) = menu.input() {
                handles.push(input.focus_handle(cx));
            }
            match &menu.mode {
                Mode::Menu => handles.extend(
                    menu.actions()
                        .iter()
                        .enumerate()
                        .filter(|(_, action)| action.3)
                        .map(|(index, _)| menu.items[index].clone()),
                ),
                Mode::Open(Some(list)) if menu.pending.is_none() => {
                    handles.push(menu.items[0].clone());
                    handles.extend(menu.items.iter().skip(2).take(list.entries.len()).cloned());
                }
                _ => {
                    handles.push(menu.items[0].clone());
                    if menu.pending.is_none()
                        && !matches!(menu.mode, Mode::Open(_) | Mode::Delete { path: None, .. })
                    {
                        handles.push(menu.items[1].clone());
                    }
                }
            }
            let current = handles.iter().position(|handle| handle.is_focused(window));
            let index = match current {
                Some(i) if backwards => (i + handles.len() - 1) % handles.len(),
                Some(i) => (i + 1) % handles.len(),
                None if backwards => handles.len() - 1,
                None => 0,
            };
            handles[index].focus(window, cx);
            cx.stop_propagation();
        } else if matches!(key.key.as_str(), "enter" | "return")
            && menu
                .input()
                .is_some_and(|input| input.read(cx).is_focused(window))
        {
            self.confirm_workspace_dialog(window, cx);
            cx.stop_propagation();
        }
    }
}

enum Confirm {
    Rename(String),
    Close(Vec<String>),
    New(String),
    Delete(bool),
}
