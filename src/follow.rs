//! Persisted preferences and pure decisions for local Herdr companion tabs.

use crate::herdr::{AgentTarget, Snapshot};
use crate::layout::{Tab, TabIcon};
use crate::process_monitor::ProcessIdentity;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitPolicy {
    Keep,
    CloseUnused,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct FollowSettings {
    pub enabled: bool,
    pub open_herdr_window: bool,
    // Applies only to local companion tabs, never to Herdr's terminals.
    pub exit_policy: ExitPolicy,
    pub socket_path: Option<String>,
}

impl Default for FollowSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            open_herdr_window: true,
            exit_policy: ExitPolicy::CloseUnused,
            socket_path: None,
        }
    }
}

impl FollowSettings {
    pub fn socket_path(&self) -> String {
        self.socket_path
            .clone()
            .or_else(|| std::env::var("MONOTERM_HERDR_SOCKET").ok())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| {
                std::env::var("HOME")
                    .map(|home| format!("{home}/.config/herdr/herdr.sock"))
                    .unwrap_or_default()
            })
    }

    pub fn load() -> (Self, Option<String>) {
        let path = match crate::appearance::config_directory() {
            Ok(directory) => directory.join("follow.json"),
            Err(error) => return (Self::default(), Some(error.to_string())),
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Self::default(), None);
            }
            Err(error) => {
                return (
                    Self::default(),
                    Some(format!("Could not read {}: {error}", path.display())),
                );
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(settings) => (settings, None),
            Err(error) => (
                Self::default(),
                Some(format!("Could not decode {}: {error}", path.display())),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let directory = crate::appearance::config_directory().map_err(|error| error.to_string())?;
        let mut bytes = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        crate::appearance::write_atomic(&directory.join("follow.json"), &bytes)
            .map_err(|error| format!("Could not save follow settings: {error}"))
    }
}

/// Apply decisions on the UI thread. Create makes and selects a whole local tab;
/// Select reuses that tab without changing any of its panes or shell state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Create(Box<AgentTarget>),
    Select(usize),
    /// Check the local shell is idle before closing, then acknowledge retirement.
    Retire(usize),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct AgentKey {
    source: String,
    terminal_id: String,
    identity: ProcessIdentity,
}

impl From<&AgentTarget> for AgentKey {
    fn from(target: &AgentTarget) -> Self {
        Self {
            source: target.source.clone(),
            terminal_id: target.terminal_id.clone(),
            identity: target.identity.clone(),
        }
    }
}

struct Companion {
    target: AgentTarget,
    tab_id: usize,
    fallback_icon: TabIcon,
    used: bool,
}

/// In-memory associations only. The caller owns local tabs and all I/O.
///
/// Set the source when starting/replacing the observer, and pass None while
/// disconnected. Apply Create synchronously and call register_created on success.
/// Retire may repeat while an asynchronous idle check is pending; only acknowledge
/// it after rechecking activity, settings, pin state, and local shell/input state.
#[derive(Default)]
pub struct Controller {
    source: Option<String>,
    companions: HashMap<AgentKey, Companion>,
    suppressed: HashSet<AgentKey>,
    confirmed_dead: HashSet<ProcessIdentity>,
    selected: Option<usize>,
    pinned: bool,
    resume_requested: bool,
}

impl Controller {
    /// Forget associations, not terminals. Discard the old snapshot with its
    /// observer: death confirmations do not carry a source in the wire snapshot.
    pub fn set_source(&mut self, source: &str) {
        if self.source.as_deref() == Some(source) {
            return;
        }
        self.source = Some(source.to_owned());
        self.companions.clear();
        self.suppressed.clear();
        self.confirmed_dead.clear();
        self.selected = None;
        self.resume_requested = false;
    }

    pub fn pin(&mut self) {
        self.pinned = true;
        self.resume_requested = false;
        self.selected = None;
    }

    /// Local navigation pauses follow only when the selected tab changes.
    pub fn manual_tab_change(&mut self, previous: Option<usize>, next: Option<usize>) {
        if previous != next {
            self.pin();
        }
    }

    /// Allow one Create/Select when an eligible focused agent is available, even
    /// if active. The request survives disconnects until used, pinned, or disabled.
    /// Retirement always waits until inactive. Resuming permits closed companions
    /// to be created again, but never revives a confirmed-dead process incarnation.
    pub fn resume(&mut self) {
        self.pinned = false;
        self.suppressed.clear();
        self.selected = None;
        self.resume_requested = true;
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// Input, manual splits, resizing, or other local edits permanently protect
    /// the entire companion tab without pausing follow.
    pub fn mark_used(&mut self, tab_id: usize) {
        if let Some(companion) = self.companions.values_mut().find(|c| c.tab_id == tab_id) {
            companion.used = true;
        }
    }

    pub fn is_companion(&self, tab_id: usize) -> bool {
        self.target_for_tab(tab_id).is_some()
    }

    /// Whether local use protects this tab, including before its agent exits.
    pub fn is_retained(&self, tab_id: usize) -> bool {
        self.companions
            .values()
            .any(|companion| companion.tab_id == tab_id && companion.used)
    }

    pub fn target_for_tab(&self, tab_id: usize) -> Option<&AgentTarget> {
        self.companions
            .values()
            .find(|companion| companion.tab_id == tab_id)
            .map(|companion| &companion.target)
    }

    /// Suppress a closed tab until Resume. The caller separately reports whether
    /// the selected tab changed; closing a background tab must not pause follow.
    pub fn close_tab(&mut self, tab_id: usize) {
        if self.selected == Some(tab_id) {
            self.selected = None;
        }
        if let Some(key) = self.key_for_tab(tab_id) {
            self.companions.remove(&key);
            self.suppressed.insert(key);
        }
    }

    pub fn tab_icon(&self, tab: &Tab) -> Option<&'static str> {
        tab.appearance.icon.map(TabIcon::name).or_else(|| {
            self.companions
                .values()
                .find(|companion| companion.tab_id == tab.id)
                .map(|companion| match companion.target.agent.as_str() {
                    "claude" => "agent-claude",
                    "codex" => "agent-codex",
                    _ => companion.fallback_icon.name(),
                })
        })
    }

    /// Register a successfully created and selected tab. A stale or duplicate
    /// result cannot replace an existing association or erase its used flag.
    pub fn register_created(&mut self, target: &AgentTarget, tab_id: usize) -> bool {
        let key = AgentKey::from(target);
        if self.source.as_deref() != Some(target.source.as_str())
            || self.suppressed.contains(&key)
            || self.confirmed_dead.contains(&target.identity)
        {
            return false;
        }
        if let Some(companion) = self.companions.get(&key) {
            return companion.tab_id == tab_id;
        }
        if self.is_companion(tab_id) {
            return false;
        }
        // Choose once per association so redraws, renames and reconnects keep it.
        let random = std::collections::hash_map::RandomState::new().hash_one(&key);
        let fallback_icon = TabIcon::ALL[random as usize % TabIcon::ALL.len()];
        self.companions.insert(
            key,
            Companion {
                target: target.clone(),
                tab_id,
                fallback_icon,
                used: false,
            },
        );
        self.selected = Some(tab_id);
        true
    }

    /// Call immediately before removing a tab after the caller's idle checks.
    /// False means the association changed or local use now protects the tab.
    pub fn acknowledge_retired(&mut self, tab_id: usize) -> bool {
        let Some(key) = self.key_for_tab(tab_id) else {
            return false;
        };
        if self.pinned || self.companions[&key].used || !self.confirmed_dead.contains(&key.identity)
        {
            return false;
        }
        self.companions.remove(&key);
        if self.selected == Some(tab_id) {
            self.selected = None;
        }
        true
    }

    fn key_for_tab(&self, tab_id: usize) -> Option<AgentKey> {
        self.companions
            .iter()
            .find(|(_, companion)| companion.tab_id == tab_id)
            .map(|(key, _)| key.clone())
    }

    pub fn evaluate(
        &mut self,
        snapshot: Option<&Snapshot>,
        active_window: bool,
        settings: &FollowSettings,
    ) -> Vec<Action> {
        if !settings.enabled {
            self.resume_requested = false;
        }
        let Some(snapshot) = snapshot else {
            return Vec::new();
        };
        self.confirmed_dead
            .extend(snapshot.confirmed_dead.iter().cloned());
        for target in &snapshot.agents {
            if let Some(companion) = self.companions.get_mut(&AgentKey::from(target)) {
                companion.target = target.clone();
            }
        }
        if !settings.enabled || self.pinned {
            return Vec::new();
        }

        let mut actions = Vec::new();
        if !active_window && settings.exit_policy == ExitPolicy::CloseUnused {
            let mut retiring: Vec<_> = self
                .companions
                .values()
                .filter(|companion| {
                    !companion.used && self.confirmed_dead.contains(&companion.target.identity)
                })
                .map(|companion| companion.tab_id)
                .collect();
            retiring.sort_unstable();
            actions.extend(retiring.into_iter().map(Action::Retire));
        }
        if active_window && !self.resume_requested {
            return actions;
        }
        let Some(target) = snapshot.agents.iter().find(|target| {
            Some(target.pane_id.as_str()) == snapshot.focused_pane_id.as_deref()
                && self.source.as_deref() == Some(target.source.as_str())
                && !self.confirmed_dead.contains(&target.identity)
        }) else {
            return actions;
        };
        let key = AgentKey::from(target);
        if self.suppressed.contains(&key) {
            return actions;
        }
        self.resume_requested = false;
        if let Some(companion) = self.companions.get(&key) {
            if self.selected != Some(companion.tab_id) {
                self.selected = Some(companion.tab_id);
                actions.push(Action::Select(companion.tab_id));
            }
        } else {
            actions.push(Action::Create(Box::new(target.clone())));
        }
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> FollowSettings {
        FollowSettings {
            enabled: true,
            ..FollowSettings::default()
        }
    }

    fn controller() -> Controller {
        let mut controller = Controller::default();
        controller.set_source("/tmp/herdr.sock");
        controller
    }

    fn target(id: u32) -> AgentTarget {
        AgentTarget {
            source: "/tmp/herdr.sock".into(),
            terminal_id: format!("term-{id}"),
            pane_id: format!("pane-{id}"),
            agent: "codex".into(),
            title: Some(format!("Space · Tab {id}")),
            native_session: None,
            identity: ProcessIdentity {
                pid: id,
                start_time_secs: 1,
                start_time_micros: 2,
            },
            directory: format!("/project/{id}").into(),
        }
    }

    fn snapshot(agents: &[AgentTarget], focused: Option<&AgentTarget>) -> Snapshot {
        Snapshot {
            epoch: 1,
            focused_pane_id: focused.map(|target| target.pane_id.clone()),
            agents: agents.to_vec(),
            confirmed_dead: Vec::new(),
            diagnostic: None,
        }
    }

    fn create(controller: &mut Controller, target: &AgentTarget, tab_id: usize) {
        let snapshot = snapshot(std::slice::from_ref(target), Some(target));
        assert_eq!(
            controller.evaluate(Some(&snapshot), false, &settings()),
            vec![Action::Create(Box::new(target.clone()))]
        );
        assert!(controller.register_created(target, tab_id));
        assert!(
            controller
                .evaluate(Some(&snapshot), false, &settings())
                .is_empty()
        );
    }

    #[test]
    fn stable_process_reuses_its_whole_tab_across_focus_and_metadata_changes() {
        let mut controller = controller();
        let mut first = target(10);
        let second = target(20);
        create(&mut controller, &first, 1);
        controller.mark_used(1); // A local split and its shells stay in this tab.
        create(&mut controller, &second, 2);

        first.title = Some("Renamed space · Renamed tab".into());
        first.pane_id = "moved-pane".into();
        first.directory = "/changed-directory".into();
        first.native_session = Some(crate::herdr::NativeSessionRef {
            source: "herdr:codex".into(),
            agent: "codex".into(),
            kind: "id".into(),
            value: "late-session-metadata".into(),
        });
        let mut observed = snapshot(&[first.clone(), second.clone()], Some(&first));
        observed.epoch = 2; // Reconnect does not invalidate native identities.
        assert_eq!(
            controller.evaluate(Some(&observed), false, &settings()),
            vec![Action::Select(1)]
        );
        assert_eq!(controller.target_for_tab(1), Some(&first));
        assert!(controller.is_retained(1));
        assert!(controller.is_companion(2));
        assert!(controller.register_created(&first, 1));
        assert!(controller.is_retained(1));
        assert!(!controller.register_created(&first, 3));

        observed.focused_pane_id = Some(second.pane_id.clone());
        assert_eq!(
            controller.evaluate(Some(&observed), false, &settings()),
            vec![Action::Select(2)]
        );
    }

    #[test]
    fn companion_icons_respect_overrides_and_keep_random_defaults() {
        let mut controller = controller();
        let mut agent = target(10);
        let mut tab = Tab::new(1, 1);
        assert_eq!(controller.tab_icon(&tab), None);
        create(&mut controller, &agent, tab.id);
        assert_eq!(controller.tab_icon(&tab), Some("agent-codex"));
        agent.agent = "claude".into();
        controller.evaluate(
            Some(&snapshot(std::slice::from_ref(&agent), Some(&agent))),
            false,
            &settings(),
        );
        assert_eq!(controller.tab_icon(&tab), Some("agent-claude"));
        agent.agent = "other".into();
        controller.evaluate(
            Some(&snapshot(std::slice::from_ref(&agent), Some(&agent))),
            false,
            &settings(),
        );
        let fallback = controller.tab_icon(&tab);
        assert!(
            TabIcon::ALL
                .into_iter()
                .any(|icon| Some(icon.name()) == fallback)
        );
        tab.appearance.icon = Some(TabIcon::Star);
        assert_eq!(controller.tab_icon(&tab), Some("star"));
        agent.title = Some("Renamed space · Tab".into());
        let observed = snapshot(std::slice::from_ref(&agent), Some(&agent));
        controller.manual_tab_change(Some(tab.id), Some(2));
        assert!(
            controller
                .evaluate(Some(&observed), true, &settings())
                .is_empty()
        );
        tab.appearance = Default::default();
        assert!(controller.register_created(&agent, tab.id));
        assert_eq!(controller.tab_icon(&tab), fallback);
        assert_eq!(
            controller.target_for_tab(tab.id).unwrap().title,
            agent.title
        );
    }

    #[test]
    fn process_incarnation_and_terminal_identity_prevent_reusing_another_tab() {
        let mut controller = controller();
        let first = target(10);
        create(&mut controller, &first, 1);
        let mut restarted = first.clone();
        restarted.identity.start_time_micros += 1; // Reused PID, same pane.
        create(&mut controller, &restarted, 2);
        let mut moved_terminal = restarted.clone();
        moved_terminal.terminal_id = "different-terminal".into();
        create(&mut controller, &moved_terminal, 3);
        assert_eq!(controller.target_for_tab(1), Some(&first));
        assert_eq!(controller.target_for_tab(2), Some(&restarted));
        assert_eq!(controller.target_for_tab(3), Some(&moved_terminal));
    }

    #[test]
    fn active_window_defers_actions_and_resume_overrides_focus_only_once() {
        let mut controller = controller();
        let first = target(10);
        let second = target(20);
        let first_snapshot = snapshot(std::slice::from_ref(&first), Some(&first));
        let second_snapshot = snapshot(std::slice::from_ref(&second), Some(&second));
        assert!(
            controller
                .evaluate(Some(&first_snapshot), true, &settings())
                .is_empty()
        );
        create(&mut controller, &first, 1);
        assert!(
            controller
                .evaluate(Some(&second_snapshot), true, &settings())
                .is_empty()
        );
        controller.pin();
        assert!(controller.is_pinned());
        assert!(
            controller
                .evaluate(Some(&second_snapshot), false, &settings())
                .is_empty()
        );
        controller.resume();
        assert!(!controller.is_pinned());
        assert_eq!(
            controller.evaluate(Some(&second_snapshot), true, &settings()),
            vec![Action::Create(Box::new(second.clone()))]
        );
        assert!(controller.register_created(&second, 2));
        assert!(
            controller
                .evaluate(Some(&first_snapshot), true, &settings())
                .is_empty()
        );
        assert_eq!(
            controller.evaluate(Some(&first_snapshot), false, &settings()),
            vec![Action::Select(1)]
        );
        controller.pin();
        controller.resume();
        assert_eq!(
            controller.evaluate(Some(&second_snapshot), true, &settings()),
            vec![Action::Select(2)]
        );
        let disabled = FollowSettings::default();
        controller.resume();
        assert!(
            controller
                .evaluate(Some(&first_snapshot), false, &disabled)
                .is_empty()
        );
    }

    #[test]
    fn disconnect_missing_agents_and_unfocused_agents_never_mean_death() {
        let mut controller = controller();
        let first = target(10);
        create(&mut controller, &first, 1);
        assert!(controller.evaluate(None, false, &settings()).is_empty());
        let mut missing = snapshot(&[], None);
        missing.epoch += 1;
        missing.diagnostic = Some("inspection unavailable".into());
        assert!(
            controller
                .evaluate(Some(&missing), false, &settings())
                .is_empty()
        );
        assert!(controller.is_companion(1));
        assert!(!controller.acknowledge_retired(1));
        let second = target(20);
        let unfocused = snapshot(&[first, second], None);
        assert!(
            controller
                .evaluate(Some(&unfocused), false, &settings())
                .is_empty()
        );
    }

    #[test]
    fn resume_survives_disconnect_and_missing_focus_until_used_or_cancelled() {
        let first = target(10);
        let second = target(20);
        let first_snapshot = snapshot(std::slice::from_ref(&first), Some(&first));
        let second_snapshot = snapshot(std::slice::from_ref(&second), Some(&second));
        let unfocused = snapshot(std::slice::from_ref(&first), None);
        let missing = snapshot(&[], Some(&first));
        let mut controller = controller();
        controller.resume();
        assert!(controller.evaluate(None, true, &settings()).is_empty());
        assert!(
            controller
                .evaluate(Some(&unfocused), true, &settings())
                .is_empty()
        );
        assert!(
            controller
                .evaluate(Some(&missing), true, &settings())
                .is_empty()
        );
        assert_eq!(
            controller.evaluate(Some(&first_snapshot), true, &settings()),
            vec![Action::Create(Box::new(first.clone()))]
        );
        assert!(controller.register_created(&first, 1));
        // Arrival consumes the request; the next focus change is deferred again.
        assert!(
            controller
                .evaluate(Some(&second_snapshot), true, &settings())
                .is_empty()
        );

        controller.resume();
        assert!(
            controller
                .evaluate(None, true, &FollowSettings::default())
                .is_empty()
        );
        assert!(
            controller
                .evaluate(Some(&second_snapshot), true, &settings())
                .is_empty()
        );
        controller.resume();
        assert!(controller.evaluate(None, true, &settings()).is_empty());
        controller.pin();
        assert!(
            controller
                .evaluate(Some(&second_snapshot), false, &settings())
                .is_empty()
        );
        controller.resume();
        assert_eq!(
            controller.evaluate(Some(&second_snapshot), true, &settings()),
            vec![Action::Create(Box::new(second))]
        );
    }

    #[test]
    fn confirmed_death_retires_only_unused_tabs_and_waits_for_acknowledgement() {
        let mut controller = controller();
        let used = target(10);
        let unused = target(20);
        create(&mut controller, &used, 1);
        create(&mut controller, &unused, 2);
        controller.mark_used(1);
        let mut dead = snapshot(&[], None);
        dead.confirmed_dead = vec![used.identity.clone(), unused.identity.clone()];
        let expected = vec![Action::Retire(2)];
        assert_eq!(
            controller.evaluate(Some(&dead), false, &settings()),
            expected
        );
        assert!(controller.is_companion(2));
        assert_eq!(
            controller.evaluate(Some(&dead), false, &settings()),
            expected
        );
        assert!(!controller.acknowledge_retired(1));
        assert!(controller.acknowledge_retired(2));
        assert!(!controller.is_companion(2));
        assert!(controller.is_retained(1));
        assert!(
            controller
                .evaluate(Some(&dead), false, &settings())
                .is_empty()
        );

        // Even a stale inventory or explicit resume cannot revive the dead one.
        controller.resume();
        let stale = snapshot(std::slice::from_ref(&unused), Some(&unused));
        assert!(
            controller
                .evaluate(Some(&stale), false, &settings())
                .is_empty()
        );
        assert!(!controller.register_created(&unused, 3));
    }

    #[test]
    fn death_waits_for_inactivity_unpin_and_close_unused_policy() {
        let mut controller = controller();
        let target = target(10);
        create(&mut controller, &target, 1);
        let mut dead = snapshot(&[], None);
        dead.confirmed_dead.push(target.identity.clone());
        controller.resume();
        assert!(
            controller
                .evaluate(Some(&dead), true, &settings())
                .is_empty()
        );
        let empty = snapshot(&[], None); // Confirmation must survive deferral.
        let keep = FollowSettings {
            exit_policy: ExitPolicy::Keep,
            ..settings()
        };
        assert!(controller.evaluate(Some(&empty), false, &keep).is_empty());
        assert!(
            controller
                .evaluate(Some(&empty), false, &FollowSettings::default())
                .is_empty()
        );
        controller.pin();
        assert!(
            controller
                .evaluate(Some(&empty), false, &settings())
                .is_empty()
        );
        assert!(!controller.acknowledge_retired(1));
        controller.resume();
        assert_eq!(
            controller.evaluate(Some(&empty), false, &settings()),
            vec![Action::Retire(1)]
        );
        // User input arriving during the caller's idle check cancels retirement.
        controller.mark_used(1);
        assert!(!controller.acknowledge_retired(1));
        assert!(
            controller
                .evaluate(Some(&empty), false, &settings())
                .is_empty()
        );
        assert!(controller.is_retained(1));
    }

    #[test]
    fn source_reset_leaves_existing_tabs_outside_automatic_control() {
        let mut controller = controller();
        let first = target(10);
        create(&mut controller, &first, 1);
        controller.set_source(&first.source);
        assert!(controller.is_companion(1));
        controller.set_source("/tmp/other.sock");
        assert!(!controller.is_companion(1));
        assert!(!controller.register_created(&first, 2));
        let stale = snapshot(std::slice::from_ref(&first), Some(&first));
        assert!(
            controller
                .evaluate(Some(&stale), false, &settings())
                .is_empty()
        );
        let mut other = first.clone();
        other.source = "/tmp/other.sock".into();
        create(&mut controller, &other, 2);
        let mut dead = snapshot(&[], None);
        dead.confirmed_dead.push(first.identity.clone());
        assert_eq!(
            controller.evaluate(Some(&dead), false, &settings()),
            vec![Action::Retire(2)]
        );
        assert!(!controller.acknowledge_retired(1));
    }

    #[test]
    fn manually_closed_companion_stays_suppressed_until_explicit_resume() {
        let mut controller = controller();
        let target = target(10);
        create(&mut controller, &target, 1);
        controller.close_tab(1);
        controller.manual_tab_change(Some(1), None);
        assert!(controller.is_pinned());
        assert!(!controller.is_companion(1));
        assert!(!controller.register_created(&target, 2));
        let observed = snapshot(std::slice::from_ref(&target), Some(&target));
        assert!(
            controller
                .evaluate(Some(&observed), false, &settings())
                .is_empty()
        );
        controller.resume();
        assert_eq!(
            controller.evaluate(Some(&observed), true, &settings()),
            vec![Action::Create(Box::new(target.clone()))]
        );
        assert!(controller.register_created(&target, 2));
        assert!(controller.is_companion(2));
    }

    #[test]
    fn editing_or_reselecting_the_same_tab_preserves_follow_and_retention() {
        let mut controller = controller();
        let first = target(10);
        let second = target(20);
        create(&mut controller, &first, 1);
        controller.mark_used(1);
        controller.manual_tab_change(Some(1), Some(1));
        assert!(!controller.is_pinned());
        assert!(controller.is_retained(1));

        let mut observed = snapshot(std::slice::from_ref(&second), Some(&second));
        assert!(
            controller
                .evaluate(Some(&observed), true, &settings())
                .is_empty()
        );
        assert_eq!(
            controller.evaluate(Some(&observed), false, &settings()),
            vec![Action::Create(Box::new(second.clone()))]
        );
        assert!(controller.register_created(&second, 2));
        observed.confirmed_dead.push(first.identity.clone());
        assert!(
            controller
                .evaluate(Some(&observed), false, &settings())
                .is_empty()
        );
        assert!(!controller.acknowledge_retired(1));
    }

    #[test]
    fn only_a_changed_local_tab_pauses_follow_until_resume() {
        for (previous, next) in [(None, Some(1)), (Some(1), Some(2)), (Some(1), None)] {
            let mut controller = controller();
            controller.manual_tab_change(previous, next);
            assert!(controller.is_pinned());
            controller.manual_tab_change(next, next);
            assert!(controller.is_pinned());
            controller.resume();
            controller.manual_tab_change(next, next);
            assert!(!controller.is_pinned());
        }
    }

    #[test]
    fn closing_a_background_companion_preserves_follow_and_suppresses_recreation() {
        let mut controller = controller();
        let first = target(10);
        let second = target(20);
        create(&mut controller, &first, 1);
        create(&mut controller, &second, 2);
        controller.close_tab(1);
        controller.manual_tab_change(Some(2), Some(2));
        assert!(!controller.is_pinned());
        assert!(controller.is_companion(2));
        assert!(!controller.is_companion(1));
        let closed = snapshot(std::slice::from_ref(&first), Some(&first));
        assert!(
            controller
                .evaluate(Some(&closed), false, &settings())
                .is_empty()
        );
        let current = snapshot(std::slice::from_ref(&second), Some(&second));
        assert!(
            controller
                .evaluate(Some(&current), false, &settings())
                .is_empty()
        );
        controller.resume();
        assert_eq!(
            controller.evaluate(Some(&closed), false, &settings()),
            vec![Action::Create(Box::new(first))]
        );
    }

    #[test]
    fn old_preferences_keep_the_json_socket_and_exit_policy() {
        let original = r#"{"enabled":true,"exit_policy":"keep","socket_path":"/tmp/herdr.sock"}"#;
        let settings: FollowSettings = serde_json::from_str(original).unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.exit_policy, ExitPolicy::Keep);
        assert!(settings.open_herdr_window);
        assert_eq!(settings.socket_path(), "/tmp/herdr.sock");
        assert_eq!(
            serde_json::from_str::<FollowSettings>(&serde_json::to_string(&settings).unwrap())
                .unwrap(),
            settings
        );
    }

    #[test]
    fn missing_fields_use_backwards_compatible_defaults() {
        let settings: FollowSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, FollowSettings::default());
        let settings: FollowSettings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.exit_policy, ExitPolicy::CloseUnused);
        assert!(settings.open_herdr_window);
        assert_eq!(settings.socket_path, None);
    }

    #[test]
    fn window_preference_round_trips_without_disabling_companion_follow() {
        let settings: FollowSettings = serde_json::from_str(
            r#"{"enabled":true,"open_herdr_window":false,"socket_path":"/tmp/herdr.sock"}"#,
        )
        .unwrap();
        let restored: FollowSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        assert!(!restored.open_herdr_window);
        let mut controller = controller();
        let target = target(10);
        let snapshot = snapshot(std::slice::from_ref(&target), Some(&target));
        assert_eq!(
            controller.evaluate(Some(&snapshot), false, &restored),
            vec![Action::Create(Box::new(target))]
        );
    }
}
