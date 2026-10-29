//! Local companion following and the single reusable Herdr workspace window.

use crate::{
    TerminalApp,
    follow::{Action, ExitPolicy},
    herdr::{self, AgentTarget},
    herdr_connection::HerdrConnection,
    herdr_view::HerdrWindow,
    layout::Tab,
    process_monitor,
    session::Session,
};
use gpui::{
    Bounds, Context, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, point, prelude::*, px, size,
};
use std::path::Path;

impl TerminalApp {
    pub(crate) fn start_follow(&mut self, cx: &mut Context<Self>) {
        self.stop_herdr(cx);
        if !self.follow_settings.enabled {
            self.follow_status = "Off".into();
            return;
        }
        let source = self.follow_settings.socket_path();
        self.follow_controller.set_source(&source);
        self.follow_observer = match herdr::Client::start(source.clone()) {
            Ok(observer) => Some(observer),
            Err(error) => {
                self.follow_status = error;
                None
            }
        };
        match HerdrConnection::open(&source, false) {
            Ok(connection) => {
                if self.follow_observer.is_some() {
                    self.follow_status = "Connecting…".into();
                }
                self.herdr_connection = Some(cx.new(|_| connection));
            }
            Err(error) => self.follow_status = error,
        }
    }

    fn stop_herdr(&mut self, cx: &mut Context<Self>) {
        self.follow_generation = self.follow_generation.wrapping_add(1);
        self.follow_observer = None;
        self.follow_snapshot = None;
        if let Some(connection) = self.herdr_connection.take() {
            connection.update(cx, |connection, cx| {
                let _ = connection.set_surface_active(false);
                connection.disconnect();
                cx.notify();
            });
        }
        if let Some(child) = self.herdr_window.take() {
            cx.defer(move |cx| {
                let _ = child.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    pub(crate) fn save_follow(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.follow_settings.save() {
            self.follow_status = error;
        }
        cx.notify();
    }

    pub(crate) fn toggle_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_settings.enabled = !self.follow_settings.enabled;
        self.start_follow(cx);
        self.save_follow(cx);
        if self.follow_settings.enabled {
            self.follow_herdr(window, cx);
        } else {
            self.return_local(window, cx);
        }
    }

    pub(crate) fn set_follow_source(
        &mut self,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path
            .as_ref()
            .is_some_and(|path| !Path::new(path).is_absolute() || path.contains('\0'))
        {
            self.follow_status = "Use an absolute local Herdr socket path.".into();
            cx.notify();
            return;
        }
        let reopen = self
            .herdr_window
            .is_some_and(|child| child.read(cx).is_ok());
        self.follow_settings.socket_path = path;
        self.start_follow(cx);
        self.save_follow(cx);
        if reopen && self.follow_settings.enabled {
            self.open_herdr_window(window, cx);
        }
    }

    pub(crate) fn follow_herdr(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.resume_follow(window, cx);
        if self.follow_settings.open_herdr_window {
            self.open_herdr_window(window, cx);
        }
    }

    pub(crate) fn resume_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.follow_settings.enabled {
            self.follow_settings.enabled = true;
            self.start_follow(cx);
            self.save_follow(cx);
        }
        self.follow_controller.resume();
        self.dragging = None;
        self.pump_follow(window, cx);
        // Clicking the header control moves keyboard focus off the terminal view.
        if let Some(session) = self.active_session() {
            session.focus(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn return_local(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.follow_controller.pin();
        window.activate_window();
        if self.tabs.is_empty() {
            self.new_session(window, cx);
        } else {
            self.select(self.active.min(self.tabs.len() - 1), window, cx);
        }
        cx.notify();
    }

    pub(crate) fn mark_local_edit(&mut self) {
        if let Some(tab) = self.tabs.get(self.active) {
            self.follow_controller.mark_used(tab.id);
        }
    }

    pub(crate) fn pump_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(connection) = &self.herdr_connection {
            connection.update(cx, |connection, cx| {
                if connection.poll() {
                    cx.notify();
                }
            });
        }
        for tab in &self.tabs {
            if self.follow_controller.is_companion(tab.id)
                && self
                    .sessions
                    .iter()
                    .any(|session| tab.layout.contains(session.id) && session.input_epoch() > 0)
            {
                self.follow_controller.mark_used(tab.id);
            }
        }
        if let Some(update) = self
            .follow_observer
            .as_ref()
            .and_then(|client| client.take_update())
        {
            match update {
                herdr::Update::Synchronized(snapshot) => {
                    self.follow_status = snapshot
                        .diagnostic
                        .clone()
                        .unwrap_or_else(|| "Connected".into());
                    self.follow_snapshot = Some(snapshot);
                }
                herdr::Update::Disconnected(error) => {
                    self.follow_status = format!("Disconnected · {error}");
                    self.follow_snapshot = None;
                }
            }
            cx.notify();
        }
        let actions = self.follow_controller.evaluate(
            self.follow_snapshot.as_ref(),
            window.is_window_active(),
            &self.follow_settings,
        );
        for action in actions {
            match action {
                Action::Create(target) => self.create_companion(*target, window, cx),
                Action::Select(id) => {
                    if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
                        self.select_content(index, false, window, cx);
                    }
                }
                Action::Retire(id) => self.check_companion_retirement(id, window, cx),
            }
        }
    }

    fn create_companion(
        &mut self,
        target: AgentTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.len() >= 12 || self.sessions.len() >= 48 {
            let message = "Limit reached: 12 tabs / 48 panes.";
            if self.notice.as_deref() != Some(message) {
                self.notice = Some(message.into());
                cx.notify();
            }
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        let session = Session::new(
            id,
            &self.shell,
            &target.directory,
            self.typography.font_family.font(),
            cx,
        );
        if let Some(view) = &session.view {
            view.update(cx, |view, cx| {
                view.queue_output_bytes(b"Companion ready\r\n", cx);
            });
        }
        self.sessions.push(session);
        self.tabs.push(Tab::new(id, id));
        self.follow_controller.register_created(&target, id);
        self.select_content(self.tabs.len() - 1, false, window, cx);
    }

    fn check_companion_retirement(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.retiring_companions.contains(&id) {
            return;
        }
        let Some(tab) = self.tabs.iter().find(|tab| tab.id == id) else {
            return;
        };
        if tab.pane_count() != 1 {
            self.follow_controller.mark_used(id);
            return;
        }
        let Some(session) = self
            .sessions
            .iter()
            .find(|session| session.id == tab.active_pane)
        else {
            return;
        };
        let Some(process) = &session.process else {
            return;
        };
        let Some(pid) = process.pid else { return };
        let foreground = process.foreground_pgid();
        let epoch = session.input_epoch();
        let pane_id = session.id;
        let generation = self.follow_generation;
        let observer_epoch = self.follow_snapshot.as_ref().map(|snapshot| snapshot.epoch);
        self.retiring_companions.insert(id);
        cx.spawn_in(window, async move |root, cx| {
            let idle = cx
                .background_executor()
                .spawn(async move { process_monitor::shell_is_idle(pid, foreground) })
                .await;
            let _ = root.update_in(cx, |root, window, cx| {
                root.retiring_companions.remove(&id);
                if root.follow_generation != generation
                    || root.follow_snapshot.as_ref().map(|snapshot| snapshot.epoch)
                        != observer_epoch
                {
                    return;
                }
                if !matches!(idle, Ok(true)) {
                    // An occupied or unverifiable shell must survive agent exit.
                    root.follow_controller.mark_used(id);
                    return;
                }
                if window.is_window_active()
                    || !root.follow_settings.enabled
                    || root.follow_settings.exit_policy != ExitPolicy::CloseUnused
                    || root.follow_snapshot.is_none()
                    || root.follow_controller.is_pinned()
                    || root.follow_controller.is_retained(id)
                {
                    return;
                }
                let unchanged = root
                    .sessions
                    .iter()
                    .find(|session| session.id == pane_id)
                    .is_some_and(|session| {
                        session.input_epoch() == epoch
                            && session.running
                            && session.process.as_ref().is_some_and(|process| {
                                process.pid == Some(pid) && process.foreground_pgid() == foreground
                            })
                    });
                if unchanged && root.follow_controller.acknowledge_retired(id) {
                    root.remove_pane(pane_id, false, window, cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn open_herdr_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.follow_settings.enabled {
            self.resume_follow(window, cx);
        }
        if self.herdr_connection.is_none() {
            self.start_follow(cx);
        }
        let Some(connection) = self.herdr_connection.clone() else {
            cx.notify();
            return;
        };
        let owner = window
            .window_handle()
            .downcast::<Self>()
            .expect("terminal window root");
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                window.display(cx).map(|display| display.id()),
                size(px(1000.0), px(700.0)),
                cx,
            ))),
            titlebar: Some(TitlebarOptions {
                title: Some("Herdr — MonoTerm".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(12.0), px(9.0))),
            }),
            window_background: WindowBackgroundAppearance::Transparent,
            window_min_size: Some(size(px(640.0), px(360.0))),
            ..Default::default()
        };
        // Opening a window draws synchronously; release the main entity first.
        cx.defer(move |cx| {
            let Ok(preferences) = owner.read_with(cx, |root, _| {
                (
                    root.follow_settings.enabled,
                    root.herdr_connection.clone(),
                    root.herdr_window,
                    root.typography.font_family.font(),
                    root.font_size,
                    root.palette,
                    root.appearance.blur_radius,
                    root.language,
                )
            }) else {
                return;
            };
            let (enabled, current, existing, font, font_size, palette, blur_radius, language) =
                preferences;
            if !enabled || current.as_ref() != Some(&connection) {
                return;
            }
            if let Some(child) = existing
                && child
                    .update(cx, |child, window, cx| child.focus(window, cx))
                    .is_ok()
            {
                return;
            }
            let result = cx.open_window(options, |window, cx| {
                cx.new(|cx| {
                    HerdrWindow::new(
                        connection,
                        owner,
                        font,
                        font_size,
                        palette,
                        blur_radius,
                        language,
                        window,
                        cx,
                    )
                })
            });
            let _ = owner.update(cx, |root, _, cx| {
                match result {
                    Ok(child) => root.herdr_window = Some(child),
                    Err(error) => root.follow_status = format!("Open Herdr window: {error}"),
                }
                cx.notify();
            });
        });
    }

    pub(crate) fn reconnect_herdr(&mut self, cx: &mut Context<Self>) {
        if !self.follow_settings.enabled {
            return;
        }
        self.follow_generation = self.follow_generation.wrapping_add(1);
        self.follow_snapshot = None;
        self.follow_observer = None;
        match herdr::Client::start(self.follow_settings.socket_path()) {
            Ok(observer) => self.follow_observer = Some(observer),
            Err(error) => self.follow_status = error,
        }
        if let Some(connection) = &self.herdr_connection {
            connection.update(cx, |connection, cx| {
                connection.reconnect();
                cx.notify();
            });
        } else {
            self.start_follow(cx);
        }
        cx.notify();
    }

    pub(crate) fn sync_herdr_preferences(&self, cx: &mut Context<Self>) {
        let Some(child) = self.herdr_window else {
            return;
        };
        let font = self.typography.font_family.font();
        let font_size = self.font_size;
        let palette = self.palette;
        let blur_radius = self.appearance.blur_radius;
        let language = self.language;
        cx.defer(move |cx| {
            let _ = child.update(cx, |child, window, cx| {
                child.set_preferences(font, font_size, palette, blur_radius, language, window, cx);
            });
        });
    }

    pub(crate) fn finish_local_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.follow_settings.enabled
            || self
                .herdr_window
                .is_some_and(|child| child.read(cx).is_ok())
        {
            self.active = 0;
            self.root_focus.focus(window, cx);
            cx.notify();
        } else {
            cx.quit();
        }
    }

    pub(crate) fn follow_label(&self) -> &'static str {
        if !self.follow_settings.enabled {
            "Herdr follow"
        } else if self.follow_controller.is_pinned() {
            "Pinned"
        } else {
            "Following Herdr"
        }
    }

    pub(crate) fn tab_label(&self, tab: &Tab) -> String {
        if let Some(title) = &tab.appearance.title {
            return title.clone();
        }
        self.default_tab_label(tab)
    }

    pub(crate) fn default_tab_label(&self, tab: &Tab) -> String {
        match self.follow_controller.target_for_tab(tab.id) {
            Some(target) => {
                let label = target
                    .title
                    .clone()
                    .unwrap_or_else(|| crate::path_name(&target.directory));
                if self
                    .follow_snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.confirmed_dead.contains(&target.identity))
                {
                    self.language.dynamic_text(&format!("{label} · kept"))
                } else {
                    label
                }
            }
            None => format!("{} · {}", crate::shell_name(&self.shell), tab.id),
        }
    }
}
