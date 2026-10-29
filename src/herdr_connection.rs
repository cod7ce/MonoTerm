//! Nonblocking owner of MonoTerm's Herdr binary connection.
//!
//! Store this in a GPUI entity and call `poll` regularly (including while the
//! window is hidden). All socket I/O belongs to herdr-client's worker. Public
//! snapshot/surface/image fields are read-only views for the caller; a surface
//! is exposed only alongside its exact snapshot projection. Status text is
//! English; localization belongs to the GUI.
//!
//! Navigation and activation require correlated successful API responses, then
//! a matching surface. Input is never buffered here or replayed after failure.
//! Shell mutations use that same fence: accepted requests gate input at once.
//! For asynchronous clipboard/IME acquisition, capture an `InputToken` before
//! starting work and pass it to `send_input_to` when the work completes.

use herdr_client::{
    Client, ClientEvent, ConnectOptions, ConnectTarget, HostTheme, Method, SurfaceImages,
    connect_with_surface_active,
    protocol::{ClientPaneInputEvent, ClientShellSnapshot, ClientSurfaceSize, PaneSurfaceFrame},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const EVENTS_PER_POLL: usize = 32;
const INITIAL_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(8);
const FENCE_TIMEOUT: Duration = Duration::from_secs(65);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputTarget {
    Pane(String),
    Popup(String),
}

/// Opaque identity of an input destination. A token never survives a reconnect,
/// a focus round trip, a new projection, a popup transition, or a resize.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputToken {
    epoch: u64,
    boot_id: String,
    projection_revision: u64,
    target: InputTarget,
}

#[cfg(test)]
impl InputToken {
    pub fn target(&self) -> &InputTarget {
        &self.target
    }
}

#[derive(Clone)]
enum NavigationTarget {
    Pane(String),
    Tab(String),
    Workspace(String),
}

impl NavigationTarget {
    fn known(&self, snapshot: &ClientShellSnapshot) -> bool {
        match self {
            Self::Pane(id) => snapshot.panes.iter().any(|pane| &pane.pane_id == id),
            Self::Tab(id) => snapshot.tabs.iter().any(|tab| &tab.tab_id == id),
            Self::Workspace(id) => snapshot
                .workspaces
                .iter()
                .any(|space| &space.workspace_id == id),
        }
    }

    fn focused(&self, snapshot: &ClientShellSnapshot) -> bool {
        match self {
            Self::Pane(id) => snapshot.focused_pane_id.as_ref() == Some(id),
            Self::Tab(id) => snapshot.focused_tab_id.as_ref() == Some(id),
            Self::Workspace(id) => snapshot.focused_workspace_id.as_ref() == Some(id),
        }
    }
}

enum ShellMutation<'a> {
    CreateTab,
    CloseTab(&'a str),
    CreateWorkspace,
}

impl ShellMutation<'_> {
    /// Match upstream herdr-commands, capturing explicit IDs from the displayed
    /// snapshot. Never let the daemon choose a different focused workspace.
    fn request(&self, snapshot: &ClientShellSnapshot) -> Result<(Method, Value), String> {
        let workspace = snapshot.focused_workspace_id.as_ref().map(|id| {
            snapshot
                .workspaces
                .iter()
                .find(|workspace| &workspace.workspace_id == id)
                .ok_or_else(|| "Herdr focused workspace is no longer present".to_string())
        });
        match self {
            Self::CreateTab => {
                let workspace = workspace.ok_or("Herdr has no focused workspace")??;
                Ok((
                    Method::TabCreate,
                    json!({"workspace_id": workspace.workspace_id, "focus": true}),
                ))
            }
            Self::CloseTab(id) => {
                let tab = snapshot
                    .tabs
                    .iter()
                    .find(|tab| tab.tab_id == *id)
                    .ok_or("Herdr tab is no longer present")?;
                if !snapshot
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.workspace_id == tab.workspace_id)
                {
                    return Err("Herdr tab workspace is no longer present".into());
                }
                Ok((Method::TabClose, json!({"tab_id": tab.tab_id})))
            }
            Self::CreateWorkspace => {
                let mut params = json!({"focus": true});
                if let Some(workspace) = workspace {
                    params["source_workspace_id"] = json!(workspace?.workspace_id);
                }
                Ok((Method::WorkspaceCreate, params))
            }
        }
    }
}

struct Fence {
    // Also tracks shell mutations whose resulting focus is not yet known.
    focus_request: Option<String>,
    activation_request: Option<String>,
    destination: Option<NavigationTarget>,
    active: bool,
    projection: Option<u64>,
    started: Instant,
}

pub struct HerdrConnection {
    pub boot_id: Option<String>,
    pub snapshot: Option<Arc<ClientShellSnapshot>>,
    pub surface: Option<Arc<PaneSurfaceFrame>>,
    /// Last complete frame for presentation only; never used to route input.
    pub display_surface: Option<Arc<PaneSurfaceFrame>>,
    pub images: Option<Arc<SurfaceImages>>,
    pub status: String,
    socket: PathBuf,
    client: Option<Client>,
    welcomed: bool,
    enabled: bool,
    next_retry: Option<Instant>,
    backoff: Duration,
    epoch: u64,
    desired_active: bool,
    acknowledged_active: bool,
    desired_focus: bool,
    sent_focus: Option<bool>,
    geometry: ConnectOptions,
    sent_geometry: Option<ConnectOptions>,
    // A resize changing only cell pixels must still wait for a newer surface.
    resize_after: Option<u64>,
    last_surface_revision: Option<u64>,
    minimum_projection: Option<u64>,
    needs_barrier: bool,
    fence: Option<Fence>,
    host_theme: Option<HostTheme>,
    theme_sent: bool,
    revision: u64,
}

/// Derive the binary endpoint from the resolved JSON API socket used by
/// `FollowSettings::socket_path()`. Never consult Herdr's environment overrides
/// or fall back to another session when this exact endpoint is unavailable.
pub fn client_socket_path(json_socket: &str) -> Result<PathBuf, String> {
    let path = Path::new(json_socket);
    if json_socket.is_empty() || json_socket.contains('\0') {
        return Err("Herdr JSON socket path is empty or invalid".into());
    }
    let stem = path
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| "Herdr JSON socket path has no file name".to_string())?;
    let mut name = stem.to_os_string();
    name.push("-client.sock");
    Ok(path.with_file_name(name))
}

impl HerdrConnection {
    /// Returns after spawning the client worker. Transport failures are reported
    /// through `status` by `poll`, which retries with a 250 ms to 8 s backoff.
    /// The hello is inactive; activation is explicit and acknowledged.
    pub fn open(json_socket: &str, active: bool) -> Result<Self, String> {
        let mut connection = Self::new(client_socket_path(json_socket)?, active);
        connection.start(Instant::now());
        Ok(connection)
    }

    fn new(socket: PathBuf, active: bool) -> Self {
        Self {
            boot_id: None,
            snapshot: None,
            surface: None,
            display_surface: None,
            images: None,
            status: "Connecting".into(),
            socket,
            client: None,
            welcomed: false,
            enabled: true,
            next_retry: None,
            backoff: INITIAL_BACKOFF,
            epoch: 0,
            desired_active: active,
            acknowledged_active: false,
            desired_focus: active,
            sent_focus: None,
            geometry: ConnectOptions {
                surface_size: ClientSurfaceSize { cols: 80, rows: 24 },
                cell_width_px: 8,
                cell_height_px: 16,
            },
            sent_geometry: None,
            resize_after: None,
            last_surface_revision: None,
            minimum_projection: None,
            needs_barrier: active,
            fence: None,
            host_theme: None,
            theme_sent: false,
            revision: 0,
        }
    }

    fn start(&mut self, now: Instant) {
        self.next_retry = None;
        self.status = "Connecting".into();
        match connect_with_surface_active(
            ConnectTarget::Socket(self.socket.clone()),
            self.geometry,
            false,
        ) {
            Ok(client) => {
                self.client = Some(client);
                self.sent_geometry = Some(self.geometry);
            }
            Err(error) => self.failed(error.to_string(), now),
        }
    }

    /// Processes at most 32 queued events and a bounded number of control sends.
    /// Returns true when rendered state or readiness may have changed. Continue
    /// polling while disconnected so the retry deadline can be serviced.
    pub fn poll(&mut self) -> bool {
        let now = Instant::now();
        let mut changed = false;
        if self.enabled && self.next_retry.is_some_and(|due| now >= due) {
            self.start(now);
            changed = true;
        }
        for _ in 0..EVENTS_PER_POLL {
            let event = match self.client.as_ref() {
                Some(client) => client.events.try_recv(),
                None => break,
            };
            match event {
                Ok(event) => {
                    self.accept(event, now);
                    changed = true;
                }
                Err(error) => {
                    if error.is_disconnected() {
                        self.failed("Connection worker stopped".into(), now);
                        changed = true;
                    }
                    break;
                }
            }
        }
        if self
            .client
            .as_ref()
            .is_some_and(|c| c.handle.is_disconnected())
        {
            self.failed("Connection closed".into(), now);
            changed = true;
        }
        if self
            .fence
            .as_ref()
            .is_some_and(|f| now.duration_since(f.started) >= FENCE_TIMEOUT)
        {
            self.failed(
                "Timed out waiting for focus or surface acknowledgement".into(),
                now,
            );
            changed = true;
        }
        changed |= self.finish_fence();
        // Do not place control commands behind metadata we have not consumed.
        if self.events_drained() {
            changed |= self.drive(now);
        }
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn connected(&self) -> bool {
        self.welcomed
            && self
                .client
                .as_ref()
                .is_some_and(|c| !c.handle.is_disconnected())
    }

    /// Optimistic readiness of the currently published projection. Sending also
    /// drains pending events and checks the captured identity again.
    pub fn input_ready(&self) -> bool {
        self.input_token().is_some()
    }

    pub fn input_target(&self) -> Option<InputTarget> {
        self.input_token().map(|token| token.target)
    }

    pub fn input_token(&self) -> Option<InputToken> {
        if !self.connected() {
            return None;
        }
        self.projected_token()
    }

    fn projected_token(&self) -> Option<InputToken> {
        if !self.desired_active
            || !self.acknowledged_active
            || !self.desired_focus
            || self.sent_focus != Some(true)
            || self.sent_geometry != Some(self.geometry)
            || self.needs_barrier
            || self.fence.is_some()
        {
            return None;
        }
        let snapshot = self.snapshot.as_ref()?;
        let surface = self.surface.as_ref()?;
        if self.boot_id.as_deref() != Some(snapshot.boot_id.as_str())
            || surface.boot_id != snapshot.boot_id
            || surface.projection_revision != snapshot.revision
            || !self.geometry_matches(surface)
            || self
                .minimum_projection
                .is_none_or(|revision| snapshot.revision < revision)
        {
            return None;
        }
        let target = if let Some(popup) = &surface.popup {
            if popup.terminal_id.is_empty() {
                return None;
            }
            InputTarget::Popup(popup.terminal_id.clone())
        } else {
            let pane_id = snapshot.focused_pane_id.as_ref()?;
            let pane = snapshot.panes.iter().find(|p| &p.pane_id == pane_id)?;
            if snapshot.focused_tab_id.as_deref() != Some(pane.tab_id.as_str())
                || snapshot.focused_workspace_id.as_deref() != Some(pane.workspace_id.as_str())
                || !surface
                    .panes
                    .iter()
                    .any(|p| &p.pane_id == pane_id && p.focused)
            {
                return None;
            }
            InputTarget::Pane(pane_id.clone())
        };
        Some(InputToken {
            epoch: self.epoch,
            boot_id: snapshot.boot_id.clone(),
            projection_revision: snapshot.revision,
            target,
        })
    }

    /// Input from a synchronous UI event. Captures the displayed destination
    /// before polling; a focus change discovered by polling rejects this event.
    pub fn send_input(
        &mut self,
        events: impl IntoIterator<Item = ClientPaneInputEvent>,
    ) -> Result<(), String> {
        let token = self
            .input_token()
            .ok_or_else(|| "Herdr input is not ready".to_string())?;
        self.send_input_to(&token, events)
    }

    /// Send only to a still-current captured destination. A failed send is
    /// discarded, including queue-full failures. Callers must never replay it.
    pub fn send_input_to(
        &mut self,
        token: &InputToken,
        events: impl IntoIterator<Item = ClientPaneInputEvent>,
    ) -> Result<(), String> {
        self.poll();
        if !self.events_drained() || self.input_token().as_ref() != Some(token) {
            return Err("Herdr input destination changed or is not ready".into());
        }
        let handle = &self.client.as_ref().ok_or("Herdr is disconnected")?.handle;
        let result = match &token.target {
            InputTarget::Pane(id) => handle.send_input(&token.boot_id, id, events),
            InputTarget::Popup(id) => handle.send_popup_input(&token.boot_id, id, events),
        };
        result.map_err(|error| format!("Input was not queued: {error}"))
    }

    /// Navigate with the daemon's pane.focus API. The triggering click/key must
    /// not be replayed into the destination. A second navigation is rejected
    /// while the first awaits its focus and activation responses.
    pub fn focus_pane(&mut self, pane_id: &str) -> Result<(), String> {
        self.navigate(NavigationTarget::Pane(pane_id.into()))
    }

    pub fn focus_tab(&mut self, tab_id: &str) -> Result<(), String> {
        self.navigate(NavigationTarget::Tab(tab_id.into()))
    }

    pub fn focus_workspace(&mut self, workspace_id: &str) -> Result<(), String> {
        self.navigate(NavigationTarget::Workspace(workspace_id.into()))
    }

    /// Create and focus a tab in the workspace displayed when the action began.
    /// Mutations are never queued for retry or replayed after reconnecting.
    pub fn create_tab(&mut self) -> Result<(), String> {
        self.mutate_shell(ShellMutation::CreateTab)
    }

    /// Close this exact tab, including an inactive tab, only while it still
    /// belongs to the captured snapshot. Never substitute the focused tab.
    pub fn close_tab(&mut self, tab_id: &str) -> Result<(), String> {
        self.mutate_shell(ShellMutation::CloseTab(tab_id))
    }

    /// Create and focus a workspace, inheriting the captured source workspace's
    /// directory as upstream does. An empty daemon may create its first space.
    pub fn create_workspace(&mut self) -> Result<(), String> {
        self.mutate_shell(ShellMutation::CreateWorkspace)
    }

    fn mutate_shell(&mut self, mutation: ShellMutation<'_>) -> Result<(), String> {
        let snapshot = self.snapshot.clone().ok_or("Herdr has no snapshot")?;
        let epoch = self.epoch;
        let (method, params) = mutation.request(&snapshot)?;
        self.poll();
        if !self.connected() || !self.events_drained() {
            return Err("Herdr is not ready to change tabs or workspaces".into());
        }
        self.validate_mutation_snapshot(&snapshot, epoch)?;
        if self.fence.is_some() || self.needs_barrier {
            return Err("Herdr focus is awaiting acknowledgement".into());
        }
        let status = match mutation {
            ShellMutation::CreateTab => "Creating tab",
            ShellMutation::CloseTab(_) => "Closing tab",
            ShellMutation::CreateWorkspace => "Creating workspace",
        };
        self.request_focus_change(&snapshot.boot_id, method, params, None, status)
    }

    fn validate_mutation_snapshot(
        &self,
        snapshot: &ClientShellSnapshot,
        epoch: u64,
    ) -> Result<(), String> {
        if self.epoch != epoch
            || self.boot_id.as_deref() != Some(snapshot.boot_id.as_str())
            || self.snapshot.as_deref() != Some(snapshot)
        {
            return Err("Herdr mutation destination changed or is no longer present".into());
        }
        Ok(())
    }

    fn navigate(&mut self, destination: NavigationTarget) -> Result<(), String> {
        self.poll();
        if !self.connected() || !self.events_drained() {
            return Err("Herdr is not ready to change focus".into());
        }
        if self.fence.is_some() || self.needs_barrier {
            return Err("Herdr focus is awaiting acknowledgement".into());
        }
        let (boot_id, known, already_focused) = {
            let snapshot = self.snapshot.as_ref().ok_or("Herdr has no snapshot")?;
            (
                snapshot.boot_id.clone(),
                destination.known(snapshot),
                destination.focused(snapshot),
            )
        };
        if !known {
            return Err("Herdr destination is no longer present".into());
        }
        if already_focused {
            return Ok(());
        }
        let (method, params) = match &destination {
            NavigationTarget::Pane(id) => (Method::PaneFocus, json!({"pane_id": id})),
            NavigationTarget::Tab(id) => (Method::TabFocus, json!({"tab_id": id})),
            NavigationTarget::Workspace(id) => {
                (Method::WorkspaceFocus, json!({"workspace_id": id}))
            }
        };
        self.request_focus_change(
            &boot_id,
            method,
            params,
            Some(destination),
            "Changing pane focus",
        )
    }

    fn request_focus_change(
        &mut self,
        boot_id: &str,
        method: Method,
        params: Value,
        destination: Option<NavigationTarget>,
        status: &str,
    ) -> Result<(), String> {
        let handle = self
            .client
            .as_ref()
            .ok_or("Herdr is disconnected")?
            .handle
            .clone();
        let request = handle
            .request(boot_id, method, params)
            .map_err(|e| e.to_string())?;
        let activation = match handle.set_surface_active(boot_id, self.desired_active) {
            Ok(id) => id,
            Err(error) => {
                let reason = format!("Focus barrier was not queued: {error}");
                self.failed(reason.clone(), Instant::now());
                return Err(reason);
            }
        };
        // Both requests are on the worker's ordered queue; input stays gated
        // until their responses and a matching acknowledged surface arrive. The
        // old frame remains for presentation only.
        self.begin_focus_change(request, activation, destination, status);
        Ok(())
    }

    fn begin_focus_change(
        &mut self,
        request: String,
        activation_request: String,
        destination: Option<NavigationTarget>,
        status: &str,
    ) {
        self.invalidate_input();
        self.surface = None;
        self.acknowledged_active = false;
        self.fence = Some(Fence {
            focus_request: Some(request),
            activation_request: Some(activation_request),
            destination,
            active: self.desired_active,
            projection: None,
            started: Instant::now(),
        });
        self.status = status.into();
    }

    /// Persistent surface interest, also restored after a reconnect. Deactivation
    /// gates input immediately; its queued API result is still checked.
    pub fn set_surface_active(&mut self, active: bool) -> Result<(), String> {
        if self.desired_active != active {
            self.desired_active = active;
            self.needs_barrier = true;
            self.invalidate_input();
            if !active {
                self.surface = None;
                self.display_surface = None;
            }
        }
        self.drive_controls()
    }

    pub fn set_host_theme(&mut self, theme: HostTheme) -> Result<(), String> {
        if self.host_theme.as_ref() != Some(&theme) {
            self.host_theme = Some(theme);
            self.theme_sent = false;
        }
        self.drive_controls()
    }

    /// Window focus, distinct from claiming the terminal surface. Reacquiring
    /// window focus establishes another response fence before accepting input.
    pub fn set_focus(&mut self, focused: bool) -> Result<(), String> {
        if self.desired_focus != focused {
            self.desired_focus = focused;
            self.invalidate_input();
            if focused && self.desired_active {
                self.needs_barrier = true;
            }
        }
        self.drive_controls()
    }

    /// Remembers the latest desired geometry even before the first snapshot.
    /// Identical geometry is a no-op only for this connection's sent identity;
    /// reconnect resets that identity. Invalid geometry never replaces it.
    pub fn resize(&mut self, size: ClientSurfaceSize, cell: (u32, u32)) -> Result<(), String> {
        let next = ConnectOptions {
            surface_size: size,
            cell_width_px: cell.0,
            cell_height_px: cell.1,
        };
        validate_geometry(next)?;
        if self.geometry != next {
            self.geometry = next;
            self.surface = None;
            self.needs_barrier = true;
            self.invalidate_input();
        }
        self.drive_controls()
    }

    /// Cancel without joining a worker or stopping the daemon. All visible state
    /// and pending acknowledgements are invalidated immediately. Stops retries.
    pub fn disconnect(&mut self) {
        self.enabled = false;
        self.next_retry = None;
        self.clear_connection();
        self.status = "Disconnected".into();
    }

    /// Start a fresh attempt at the same exact socket. Restores only focus,
    /// geometry and surface interest, never a pane navigation or input event.
    pub fn reconnect(&mut self) {
        self.clear_connection();
        self.enabled = true;
        self.backoff = INITIAL_BACKOFF;
        self.start(Instant::now());
    }

    fn invalidate_input(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
    }

    fn clear_connection(&mut self) {
        if let Some(client) = self.client.take() {
            client.handle.disconnect();
        }
        self.invalidate_input();
        self.welcomed = false;
        self.boot_id = None;
        self.snapshot = None;
        self.surface = None;
        self.display_surface = None;
        self.images = None;
        self.theme_sent = false;
        self.fence = None;
        self.sent_focus = None;
        self.sent_geometry = None;
        self.resize_after = None;
        self.last_surface_revision = None;
        self.minimum_projection = None;
        self.acknowledged_active = false;
        self.needs_barrier = self.desired_active;
    }

    fn failed(&mut self, reason: String, now: Instant) {
        self.clear_connection();
        self.status = format!("Disconnected: {}", display_reason(&reason));
        if self.enabled {
            self.next_retry = Some(now + self.backoff);
            self.backoff = (self.backoff * 2).min(MAX_BACKOFF);
        }
    }

    fn events_drained(&self) -> bool {
        self.client.as_ref().is_some_and(|c| c.events.is_empty())
    }

    fn drive_controls(&mut self) -> Result<(), String> {
        if self.events_drained() && self.drive(Instant::now()) && self.client.is_none() {
            return Err(self.status.clone());
        }
        Ok(())
    }

    // Control preferences may be coalesced, but neither input nor navigation is.
    fn drive(&mut self, now: Instant) -> bool {
        if !self.connected() || self.boot_id.is_none() || self.fence.is_some() {
            return false;
        }
        if !self.theme_sent
            && let Some(theme) = &self.host_theme
        {
            let result = self
                .client
                .as_ref()
                .ok_or_else(|| "Herdr is disconnected".to_string())
                .and_then(|client| {
                    client
                        .handle
                        .set_host_theme(self.boot_id.as_deref().unwrap_or_default(), theme)
                        .map_err(|e| e.to_string())
                });
            if let Err(error) = result {
                self.failed(format!("Host theme was not queued: {error}"), now);
                return true;
            }
            self.theme_sent = true;
        }
        if self.sent_geometry == Some(self.geometry)
            && self.sent_focus == Some(self.desired_focus)
            && !self.needs_barrier
            && self.desired_active == self.acknowledged_active
        {
            return false;
        }
        let (Some(handle), Some(boot)) = (
            self.client.as_ref().map(|client| client.handle.clone()),
            self.boot_id.clone(),
        ) else {
            return false;
        };
        let mut changed = false;
        if self.sent_geometry != Some(self.geometry) {
            if let Err(error) = handle.resize(&boot, self.geometry) {
                self.failed(format!("Resize was not queued: {error}"), now);
                return true;
            }
            self.sent_geometry = Some(self.geometry);
            self.resize_after = self.last_surface_revision;
            self.needs_barrier = true;
            changed = true;
        }
        if self.sent_focus != Some(self.desired_focus) {
            if let Err(error) = handle.set_focus(&boot, self.desired_focus) {
                self.failed(format!("Window focus was not queued: {error}"), now);
                return true;
            }
            self.sent_focus = Some(self.desired_focus);
            changed = true;
        }
        if self.needs_barrier || self.desired_active != self.acknowledged_active {
            match handle.set_surface_active(&boot, self.desired_active) {
                Ok(id) => {
                    self.needs_barrier = false;
                    self.fence = Some(Fence {
                        focus_request: None,
                        activation_request: Some(id),
                        destination: None,
                        active: self.desired_active,
                        projection: None,
                        started: now,
                    });
                    self.status = "Waiting for surface acknowledgement".into();
                }
                Err(error) => self.failed(format!("Surface request was not queued: {error}"), now),
            }
            changed = true;
        }
        changed
    }

    fn accept(&mut self, event: ClientEvent, now: Instant) {
        match event {
            ClientEvent::Connected(welcome) => {
                self.welcomed = true;
                self.status = format!("Connected: {}", display_reason(&welcome.server_version));
            }
            ClientEvent::Snapshot(snapshot) => {
                if snapshot.boot_id.is_empty()
                    || self.snapshot.as_ref().is_some_and(|old| {
                        old.boot_id != snapshot.boot_id || old.revision > snapshot.revision
                    })
                {
                    self.failed("Snapshot identity changed".into(), now);
                    return;
                }
                if self
                    .snapshot
                    .as_ref()
                    .is_none_or(|old| old.as_ref() != snapshot.as_ref())
                {
                    self.invalidate_input();
                }
                if self.surface.as_ref().is_some_and(|surface| {
                    surface.boot_id != snapshot.boot_id
                        || surface.projection_revision != snapshot.revision
                }) {
                    self.surface = None;
                }
                self.boot_id = Some(snapshot.boot_id.clone());
                self.snapshot = Some(snapshot);
                if !self.desired_active {
                    self.backoff = INITIAL_BACKOFF;
                }
            }
            ClientEvent::Surface(surface) => {
                if self.snapshot.as_ref().is_none_or(|snapshot| {
                    surface.boot_id != snapshot.boot_id
                        || surface.projection_revision != snapshot.revision
                }) {
                    // The client already fences these; never publish a mismatched pair.
                    return;
                }
                let popup_changed = self
                    .surface
                    .as_ref()
                    .and_then(|s| s.popup.as_ref())
                    .map(|p| p.terminal_id.as_str())
                    != surface.popup.as_ref().map(|p| p.terminal_id.as_str());
                if popup_changed {
                    self.invalidate_input();
                }
                self.last_surface_revision = Some(surface.surface_revision);
                self.display_surface = Some(surface.clone());
                self.surface = Some(surface);
            }
            ClientEvent::SurfaceImages(images) => self.images = Some(images),
            ClientEvent::Response {
                request_id,
                response,
            } => {
                if let Err(reason) = self.acknowledge(&request_id, &response) {
                    self.failed(reason, now);
                }
            }
            ClientEvent::CommandRejected { reason, .. } => {
                self.failed(format!("Command rejected: {reason}"), now);
            }
            ClientEvent::Disconnected { reason, .. } => self.failed(reason, now),
            ClientEvent::VersionMismatch(_) => {
                self.status = "Herdr client and daemon versions are incompatible".into();
            }
            // Optional notifications are data only; no clipboard or process side effects.
            ClientEvent::Message(_) => {}
        }
    }

    fn acknowledge(&mut self, id: &str, response: &Value) -> Result<(), String> {
        let Some(fence) = self.fence.as_mut() else {
            return Err("Unexpected Herdr response".into());
        };
        let is_focus = fence.focus_request.as_deref() == Some(id);
        let is_activation = fence.activation_request.as_deref() == Some(id);
        if !is_focus && !is_activation {
            return Err("Herdr response does not match the pending request".into());
        }
        if response.get("id").and_then(Value::as_str) != Some(id)
            || response.get("error").is_some_and(|value| !value.is_null())
            || !response.get("result").is_some_and(Value::is_object)
        {
            return Err("Herdr rejected the focus or surface request".into());
        }
        if is_focus {
            fence.focus_request = None;
        } else {
            let result = &response["result"];
            if result["type"].as_str() != Some("client_shell_surface_set")
                || result["active"].as_bool() != Some(fence.active)
            {
                return Err("Invalid Herdr surface acknowledgement".into());
            }
            fence.projection = Some(
                result["projection_revision"]
                    .as_u64()
                    .ok_or("Herdr surface acknowledgement has no projection revision")?,
            );
            fence.activation_request = None;
        }
        Ok(())
    }

    fn geometry_matches(&self, surface: &PaneSurfaceFrame) -> bool {
        surface.frame.width == self.geometry.surface_size.cols
            && surface.frame.height == self.geometry.surface_size.rows
            && self
                .resize_after
                .is_none_or(|revision| surface.surface_revision > revision)
    }

    fn finish_fence(&mut self) -> bool {
        let (active, destination, projection) = {
            let Some(fence) = self.fence.as_ref() else {
                return false;
            };
            if fence.focus_request.is_some() || fence.activation_request.is_some() {
                return false;
            }
            let Some(projection) = fence.projection else {
                return false;
            };
            (fence.active, fence.destination.clone(), projection)
        };
        // If preferences changed while awaiting a response, complete this fence
        // without waiting for a now-unwanted surface; drive queues the new state.
        let superseded = self.needs_barrier
            || self.sent_geometry != Some(self.geometry)
            || active != self.desired_active;
        if active && !superseded {
            let Some(snapshot) = self.snapshot.as_ref() else {
                return false;
            };
            let Some(surface) = self.surface.as_ref() else {
                return false;
            };
            if snapshot.revision < projection
                || surface.boot_id != snapshot.boot_id
                || surface.projection_revision != snapshot.revision
                || !self.geometry_matches(surface)
                || destination
                    .as_ref()
                    .is_some_and(|target| !target.focused(snapshot))
            {
                return false;
            }
        }
        self.acknowledged_active = active;
        self.minimum_projection = Some(projection);
        self.fence = None;
        if !superseded {
            self.backoff = INITIAL_BACKOFF;
            self.status = if self.acknowledged_active {
                "Connected"
            } else {
                "Connected (inactive)"
            }
            .into();
        }
        true
    }
}

impl Drop for HerdrConnection {
    fn drop(&mut self) {
        if let Some(client) = &self.client {
            client.handle.disconnect();
        }
    }
}

fn validate_geometry(options: ConnectOptions) -> Result<(), String> {
    let size = options.surface_size;
    if size.cols == 0
        || size.rows == 0
        || size.cols > 4096
        || size.rows > 4096
        || u32::from(size.cols) * u32::from(size.rows) > 1_000_000
        || options.cell_width_px > 4096
        || options.cell_height_px > 4096
    {
        return Err("Herdr surface geometry is outside the supported bounds".into());
    }
    Ok(())
}

fn display_reason(reason: &str) -> String {
    reason
        .chars()
        .filter(|ch| !ch.is_control())
        .take(512)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_client::protocol::{
        ClientShellPopupSurface, FrameData, PaneSurfacePane, SurfaceRect,
    };
    use serde_json::json;

    fn snapshot() -> Arc<ClientShellSnapshot> {
        Arc::new(
            serde_json::from_str(include_str!(
                "../vendor/herdr/herdr-protocol/tests/fixtures/endpoint-snapshot-v1.json"
            ))
            .unwrap(),
        )
    }

    fn surface(snapshot: &ClientShellSnapshot, revision: u64) -> Arc<PaneSurfaceFrame> {
        let rect = SurfaceRect {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        };
        Arc::new(PaneSurfaceFrame {
            boot_id: snapshot.boot_id.clone(),
            projection_revision: snapshot.revision,
            surface_revision: revision,
            frame: FrameData {
                width: 80,
                height: 24,
                cells: vec![],
                cursor: None,
                hyperlinks: vec![],
                graphics: vec![],
            },
            panes: vec![PaneSurfacePane {
                pane_id: snapshot.focused_pane_id.clone().unwrap(),
                content_revision: 1,
                rect,
                inner_rect: rect,
                scrollbar_rect: None,
                scroll: None,
                focused: true,
                mouse_reporting: false,
                sgr_pixel_mouse: false,
                alternate_screen_active: false,
                pixel_width: 640,
                pixel_height: 384,
            }],
            splits: vec![],
            popup: None,
            graphics: Default::default(),
        })
    }

    // Reducer tests use injected events only: no default daemon or socket is opened.
    fn ready() -> HerdrConnection {
        let mut owner = HerdrConnection::new(PathBuf::from("/unused/herdr-client.sock"), true);
        let snapshot = snapshot();
        owner.boot_id = Some(snapshot.boot_id.clone());
        owner.surface = Some(surface(&snapshot, 1));
        owner.display_surface = owner.surface.clone();
        owner.last_surface_revision = Some(1);
        owner.minimum_projection = Some(snapshot.revision);
        owner.snapshot = Some(snapshot);
        owner.acknowledged_active = true;
        owner.sent_focus = Some(true);
        owner.sent_geometry = Some(owner.geometry);
        owner.needs_barrier = false;
        owner
    }

    fn fence(owner: &mut HerdrConnection, pane: Option<&str>) {
        owner.invalidate_input();
        owner.fence = Some(Fence {
            focus_request: pane.map(|_| "focus-1".into()),
            activation_request: Some("activate-1".into()),
            destination: pane.map(|id| NavigationTarget::Pane(id.into())),
            active: true,
            projection: None,
            started: Instant::now(),
        });
    }

    fn activation(revision: u64) -> Value {
        json!({"id": "activate-1", "result": {
            "type": "client_shell_surface_set", "active": true, "projection_revision": revision
        }})
    }

    #[test]
    fn exact_json_socket_derivation_does_not_select_another_session() {
        assert_eq!(
            client_socket_path("/config/herdr/sessions/work/herdr.sock").unwrap(),
            PathBuf::from("/config/herdr/sessions/work/herdr-client.sock")
        );
        assert_eq!(
            client_socket_path("/tmp/custom.name.sock").unwrap(),
            PathBuf::from("/tmp/custom.name-client.sock")
        );
        assert!(client_socket_path("").is_err());
        assert!(client_socket_path("/").is_err());
    }

    #[test]
    fn shell_mutations_use_upstream_params_and_explicit_targets() {
        let mut snapshot = (*snapshot()).clone();
        assert_eq!(
            ShellMutation::CreateTab.request(&snapshot).unwrap(),
            (
                Method::TabCreate,
                json!({"workspace_id": "w1", "focus": true})
            )
        );
        assert_eq!(
            ShellMutation::CreateWorkspace.request(&snapshot).unwrap(),
            (
                Method::WorkspaceCreate,
                json!({"source_workspace_id": "w1", "focus": true})
            )
        );
        let mut inactive = snapshot.tabs[0].clone();
        inactive.tab_id = "w1:t2".into();
        inactive.focused = false;
        snapshot.tabs.push(inactive);
        assert_eq!(
            ShellMutation::CloseTab("w1:t2").request(&snapshot).unwrap(),
            (Method::TabClose, json!({"tab_id": "w1:t2"}))
        );
        assert!(
            ShellMutation::CloseTab("missing")
                .request(&snapshot)
                .is_err()
        );
        assert!(ShellMutation::CloseTab("").request(&snapshot).is_err());
        snapshot.workspaces.clear();
        assert!(ShellMutation::CreateTab.request(&snapshot).is_err());
        assert!(ShellMutation::CreateWorkspace.request(&snapshot).is_err());
        assert!(ShellMutation::CloseTab("w1:t2").request(&snapshot).is_err());
        snapshot.focused_workspace_id = None;
        snapshot.focused_tab_id = None;
        snapshot.focused_pane_id = None;
        snapshot.tabs.clear();
        snapshot.panes.clear();
        assert_eq!(
            ShellMutation::CreateWorkspace.request(&snapshot).unwrap(),
            (Method::WorkspaceCreate, json!({"focus": true}))
        );
        assert!(ShellMutation::CreateTab.request(&snapshot).is_err());
    }

    #[test]
    fn mutations_reject_snapshots_changed_while_draining_events() {
        for (mutation, method) in [
            (ShellMutation::CreateTab, Method::TabCreate),
            (ShellMutation::CloseTab("w1:t1"), Method::TabClose),
            (ShellMutation::CreateWorkspace, Method::WorkspaceCreate),
        ] {
            let mut owner = ready();
            let captured = owner.snapshot.clone().unwrap();
            let epoch = owner.epoch;
            assert_eq!(mutation.request(&captured).unwrap().0, method);
            assert!(owner.validate_mutation_snapshot(&captured, epoch).is_ok());
            let mut next = (*captured).clone();
            next.revision += 1;
            next.focused_workspace_id = Some("w2".into());
            next.workspaces[0].workspace_id = "w2".into();
            next.tabs.clear();
            owner.accept(ClientEvent::Snapshot(Arc::new(next)), Instant::now());
            assert!(owner.validate_mutation_snapshot(&captured, epoch).is_err());
            assert!(owner.projected_token().is_none());
            assert!(owner.fence.is_none(), "a stale action must not be queued");
        }
        let mut owner = ready();
        let captured = owner.snapshot.clone().unwrap();
        let epoch = owner.epoch;
        owner.invalidate_input();
        assert!(
            owner.validate_mutation_snapshot(&captured, epoch).is_err(),
            "even the same projection must not survive an input epoch change"
        );
    }

    #[test]
    fn mutation_fence_requires_both_responses_and_a_current_surface() {
        for mutation in [
            ShellMutation::CreateTab,
            ShellMutation::CloseTab("w1:t1"),
            ShellMutation::CreateWorkspace,
        ] {
            let mut owner = ready();
            let old_token = owner.projected_token().unwrap();
            let previous = owner.display_surface.clone().unwrap();
            let (method, _) = mutation.request(owner.snapshot.as_ref().unwrap()).unwrap();
            owner.begin_focus_change(
                "mutation-1".into(),
                "activate-1".into(),
                None,
                method.as_str(),
            );
            assert!(owner.surface.is_none());
            assert!(owner.projected_token().is_none());
            assert!(Arc::ptr_eq(
                owner.display_surface.as_ref().unwrap(),
                &previous
            ));

            let mut next = (*snapshot()).clone();
            next.revision += 1;
            owner.accept(
                ClientEvent::Response {
                    request_id: "activate-1".into(),
                    response: activation(next.revision),
                },
                Instant::now(),
            );
            assert!(
                !owner.finish_fence(),
                "the mutation response is still pending"
            );
            owner.accept(
                ClientEvent::Response {
                    request_id: "mutation-1".into(),
                    response: json!({"id": "mutation-1", "result": {}}),
                },
                Instant::now(),
            );
            owner.accept(ClientEvent::Surface(previous.clone()), Instant::now());
            assert!(!owner.finish_fence(), "the old projection remains fenced");
            assert!(owner.projected_token().is_none());
            owner.accept(
                ClientEvent::Snapshot(Arc::new(next.clone())),
                Instant::now(),
            );
            owner.accept(ClientEvent::Surface(previous), Instant::now());
            assert!(owner.surface.is_none(), "stale surfaces must be ignored");
            assert!(
                !owner.finish_fence(),
                "the acknowledged snapshot needs its surface"
            );
            owner.accept(ClientEvent::Surface(surface(&next, 2)), Instant::now());
            assert!(owner.finish_fence());
            assert_ne!(owner.projected_token().unwrap(), old_token);
        }
    }

    #[test]
    fn failed_mutations_clear_the_fence_and_never_restore_old_input() {
        for mutation in [
            ShellMutation::CreateTab,
            ShellMutation::CloseTab("w1:t1"),
            ShellMutation::CreateWorkspace,
        ] {
            let mut owner = ready();
            let old_token = owner.projected_token().unwrap();
            let (method, _) = mutation.request(owner.snapshot.as_ref().unwrap()).unwrap();
            owner.begin_focus_change(
                "mutation-1".into(),
                "activate-1".into(),
                None,
                method.as_str(),
            );
            let now = Instant::now();
            owner.accept(
                ClientEvent::Response {
                    request_id: "mutation-1".into(),
                    response: json!({"id": "mutation-1", "error": {"code": "not_found"}}),
                },
                now,
            );
            assert!(owner.fence.is_none());
            assert!(owner.snapshot.is_none() && owner.surface.is_none());
            assert!(owner.display_surface.is_none());
            assert!(owner.projected_token().is_none());
            assert_ne!(owner.epoch, old_token.epoch);
            assert_eq!(owner.next_retry, Some(now + INITIAL_BACKOFF));
            assert!(owner.status.starts_with("Disconnected:"));
            // Retrying restores preferences only; no mutation request survives.
            assert!(owner.needs_barrier);
        }
    }

    #[test]
    fn invalid_or_unavailable_mutations_preserve_input_and_presentation() {
        let mut owner = ready();
        let token = owner.projected_token().unwrap();
        let display = owner.display_surface.clone().unwrap();
        assert!(owner.close_tab("missing").is_err());
        assert!(owner.close_tab("").is_err());
        assert_eq!(owner.projected_token(), Some(token.clone()));
        assert!(owner.fence.is_none());
        // Without a transport nothing is queued, so input remains usable.
        for result in [
            owner.create_tab(),
            owner.close_tab("w1:t1"),
            owner.create_workspace(),
        ] {
            assert!(result.is_err());
            assert_eq!(owner.projected_token(), Some(token.clone()));
            assert!(Arc::ptr_eq(
                owner.display_surface.as_ref().unwrap(),
                &display
            ));
            assert!(owner.fence.is_none());
        }
    }

    #[test]
    fn focus_requires_both_responses_and_the_acknowledged_projection() {
        let mut owner = ready();
        fence(&mut owner, Some("w1:p2"));
        let mut next = (*snapshot()).clone();
        next.revision += 1;
        next.focused_pane_id = Some("w1:p2".into());
        next.panes[0].pane_id = "w1:p2".into();
        owner.accept(
            ClientEvent::Snapshot(Arc::new(next.clone())),
            Instant::now(),
        );
        owner.accept(ClientEvent::Surface(surface(&next, 2)), Instant::now());
        owner
            .acknowledge("activate-1", &activation(next.revision))
            .unwrap();
        assert!(
            !owner.finish_fence(),
            "activation alone must not acknowledge focus"
        );
        assert!(owner.projected_token().is_none());
        owner
            .acknowledge("focus-1", &json!({"id":"focus-1", "result":{}}))
            .unwrap();
        assert!(owner.finish_fence());
        assert_eq!(
            owner.projected_token().unwrap().target(),
            &InputTarget::Pane("w1:p2".into())
        );
    }

    #[test]
    fn workspace_and_tab_navigation_wait_for_the_requested_destination() {
        for destination in [
            NavigationTarget::Workspace("w2".into()),
            NavigationTarget::Tab("w2:t1".into()),
        ] {
            let mut owner = ready();
            fence(&mut owner, Some("unused"));
            owner.fence.as_mut().unwrap().destination = Some(destination.clone());
            let revision = owner.snapshot.as_ref().unwrap().revision;
            owner
                .acknowledge("focus-1", &json!({"id":"focus-1", "result":{}}))
                .unwrap();
            owner
                .acknowledge("activate-1", &activation(revision))
                .unwrap();
            assert!(
                !owner.finish_fence(),
                "an old focused destination must stay fenced"
            );
            assert!(owner.projected_token().is_none());
            assert!(owner.display_surface.is_some());
            let mut next = (*owner.snapshot.clone().unwrap()).clone();
            next.revision += 1;
            match destination {
                NavigationTarget::Workspace(id) => next.focused_workspace_id = Some(id),
                NavigationTarget::Tab(id) => next.focused_tab_id = Some(id),
                NavigationTarget::Pane(_) => unreachable!(),
            }
            next.panes[0].workspace_id = next.focused_workspace_id.clone().unwrap();
            next.panes[0].tab_id = next.focused_tab_id.clone().unwrap();
            owner.accept(
                ClientEvent::Snapshot(Arc::new(next.clone())),
                Instant::now(),
            );
            assert!(
                !owner.finish_fence(),
                "the new snapshot still needs its surface"
            );
            owner.accept(ClientEvent::Surface(surface(&next, 2)), Instant::now());
            assert!(owner.finish_fence());
            assert!(owner.projected_token().is_some());
        }
    }

    #[test]
    fn response_errors_and_missing_projection_never_open_input() {
        let mut owner = ready();
        fence(&mut owner, None);
        for response in [
            json!({"id":"activate-1", "error":{"code":"endpoint_busy"}}),
            json!({"id":"activate-1", "result":{"type":"client_shell_surface_set", "active":true}}),
            json!({"id":"wrong", "result":{}}),
        ] {
            assert!(owner.acknowledge("activate-1", &response).is_err());
            assert!(!owner.finish_fence());
            assert!(owner.projected_token().is_none());
        }
    }

    #[test]
    fn ordinary_content_surface_keeps_input_identity_stable() {
        let mut owner = ready();
        let token = owner.projected_token().unwrap();
        owner.accept(
            ClientEvent::Surface(surface(&snapshot(), 2)),
            Instant::now(),
        );
        assert_eq!(owner.projected_token(), Some(token));
    }

    #[test]
    fn new_projection_and_popup_round_trip_invalidate_captured_input() {
        let mut owner = ready();
        let old = owner.projected_token().unwrap();
        let mut popup_surface = (*owner.surface.clone().unwrap()).clone();
        popup_surface.surface_revision = 2;
        popup_surface.popup = Some(Box::new(ClientShellPopupSurface {
            terminal_id: "popup-1".into(),
            title: String::new(),
            width: None,
            height: None,
            frame: popup_surface.frame.clone(),
            mouse_reporting: false,
            sgr_pixel_mouse: false,
            pixel_width: 640,
            pixel_height: 384,
        }));
        owner.accept(
            ClientEvent::Surface(Arc::new(popup_surface)),
            Instant::now(),
        );
        let popup = owner.projected_token().unwrap();
        assert_eq!(popup.target(), &InputTarget::Popup("popup-1".into()));
        assert_ne!(popup, old);
        owner.accept(
            ClientEvent::Surface(surface(&snapshot(), 3)),
            Instant::now(),
        );
        assert_ne!(
            owner.projected_token().unwrap(),
            old,
            "returning to a pane must not revive a token"
        );
        let mut next = (*snapshot()).clone();
        next.revision += 1;
        owner.accept(
            ClientEvent::Snapshot(Arc::new(next.clone())),
            Instant::now(),
        );
        assert!(owner.surface.is_none());
        assert!(owner.projected_token().is_none());
        owner.accept(
            ClientEvent::Surface(surface(&snapshot(), 4)),
            Instant::now(),
        );
        assert!(
            owner.surface.is_none(),
            "old projection must not be exposed"
        );
        owner.accept(ClientEvent::Surface(surface(&next, 5)), Instant::now());
        assert!(owner.projected_token().is_some());
    }

    #[test]
    fn switching_keeps_last_frame_visible_without_allowing_stale_input() {
        let mut owner = ready();
        let previous = owner.display_surface.clone().unwrap();
        let mut next = (*snapshot()).clone();
        next.revision += 1;
        owner.accept(
            ClientEvent::Snapshot(Arc::new(next.clone())),
            Instant::now(),
        );
        assert!(owner.surface.is_none());
        assert!(owner.projected_token().is_none());
        assert!(Arc::ptr_eq(
            owner.display_surface.as_ref().unwrap(),
            &previous
        ));
        owner.accept(ClientEvent::Surface(surface(&next, 2)), Instant::now());
        assert_eq!(
            owner.display_surface.as_ref().unwrap().projection_revision,
            next.revision
        );
        owner.resize(owner.geometry.surface_size, (10, 20)).unwrap();
        assert!(owner.surface.is_none());
        assert!(owner.display_surface.is_some());
        assert!(owner.projected_token().is_none());
        owner.disconnect();
        assert!(owner.display_surface.is_none());
    }

    #[test]
    fn resize_identity_includes_cell_pixels_and_surface_generation() {
        let mut owner = ready();
        let old = owner.projected_token().unwrap();
        let geometry = owner.geometry;
        owner.resize(geometry.surface_size, (9, 18)).unwrap();
        assert!(owner.projected_token().is_none());
        assert_ne!(owner.sent_geometry, Some(owner.geometry));
        owner.sent_geometry = Some(owner.geometry);
        owner.resize_after = Some(1);
        owner.needs_barrier = false;
        fence(&mut owner, None);
        owner.acknowledge("activate-1", &activation(7)).unwrap();
        owner.accept(
            ClientEvent::Surface(surface(&snapshot(), 1)),
            Instant::now(),
        );
        assert!(
            !owner.finish_fence(),
            "same dimensions from before resize are insufficient"
        );
        owner.accept(
            ClientEvent::Surface(surface(&snapshot(), 2)),
            Instant::now(),
        );
        assert!(owner.finish_fence());
        assert_ne!(owner.projected_token().unwrap(), old);
        let desired = owner.geometry;
        assert!(
            owner
                .resize(ClientSurfaceSize { cols: 0, rows: 24 }, (9, 18))
                .is_err()
        );
        assert_eq!(
            owner.geometry, desired,
            "invalid geometry must not poison reconnect"
        );
    }

    #[test]
    fn disconnect_clears_projection_images_fences_and_resize_identity() {
        let mut owner = ready();
        owner.images = Some(Arc::new(SurfaceImages::default()));
        fence(&mut owner, Some("w1:p2"));
        owner.disconnect();
        assert!(!owner.connected());
        assert!(!owner.input_ready());
        assert!(owner.snapshot.is_none() && owner.surface.is_none() && owner.images.is_none());
        assert!(owner.boot_id.is_none() && owner.fence.is_none());
        assert!(owner.sent_geometry.is_none() && owner.sent_focus.is_none());
        assert!(!owner.enabled && owner.next_retry.is_none());
        assert!(
            owner
                .send_input([ClientPaneInputEvent::TextCommit("do not replay".into())])
                .is_err()
        );
    }

    #[test]
    fn boot_change_invalidates_everything_and_retry_delay_is_bounded() {
        let mut owner = ready();
        let mut next = (*snapshot()).clone();
        next.boot_id = "other-boot".into();
        let now = Instant::now();
        owner.accept(ClientEvent::Snapshot(Arc::new(next)), now);
        assert!(owner.snapshot.is_none() && owner.surface.is_none());
        assert_eq!(owner.next_retry, Some(now + INITIAL_BACKOFF));
        for _ in 0..12 {
            owner.failed("unavailable".into(), now);
        }
        assert_eq!(owner.next_retry, Some(now + MAX_BACKOFF));
        assert_eq!(owner.backoff, MAX_BACKOFF);
    }

    #[test]
    fn window_focus_and_inactive_surface_gate_input_immediately() {
        let mut owner = ready();
        let before = owner.projected_token().unwrap();
        owner.set_focus(false).unwrap();
        assert!(owner.projected_token().is_none());
        owner.set_focus(true).unwrap();
        assert!(
            owner.projected_token().is_none(),
            "refocus must await a response fence"
        );
        owner.needs_barrier = false;
        assert_ne!(owner.projected_token().unwrap(), before);
        owner.set_surface_active(false).unwrap();
        assert!(owner.projected_token().is_none());
    }
}
