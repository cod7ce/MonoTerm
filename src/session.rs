use gpui::{App, Entity, FocusHandle, Font, Window, prelude::*};
use gpui_ghostty_terminal::{
    TerminalConfig, TerminalSession,
    view::{TerminalInput, TerminalView},
};
use portable_pty::PtySize;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

use crate::{
    pty::{Output, PtyProcess},
    ui,
};

pub struct Session {
    pub id: usize,
    pub directory: PathBuf,
    pub view: Option<Entity<TerminalView>>,
    pub focus: FocusHandle,
    pub process: Option<PtyProcess>,
    pub status: String,
    pub notice: Option<String>,
    pub running: bool,
    pub grid: (u16, u16),
    pixel_size: (u16, u16),
    input_error: Arc<Mutex<Option<String>>>,
    input_epoch: Arc<AtomicU64>,
    font: Font,
}

impl Session {
    pub fn new(id: usize, shell: &str, directory: &Path, font: Font, cx: &mut App) -> Self {
        let mut session = Self {
            id,
            directory: directory.to_path_buf(),
            view: None,
            focus: cx.focus_handle(),
            process: None,
            status: "Starting…".into(),
            notice: None,
            running: false,
            grid: (80, 24),
            pixel_size: (0, 0),
            input_error: Arc::new(Mutex::new(None)),
            input_epoch: Arc::new(AtomicU64::new(0)),
            font,
        };
        session.start(shell, directory, cx);
        session
    }

    pub fn set_font(&mut self, font: Font, cx: &mut App) {
        if self.font == font {
            return;
        }
        self.font = font.clone();
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.set_font(font, cx));
        }
    }

    pub fn start(&mut self, shell: &str, directory: &Path, cx: &mut App) {
        // Release the input callback before shutting down its PTY writer.
        self.view.take();
        self.process.take();
        self.running = false;
        self.notice = None;
        self.grid = (80, 24);
        self.pixel_size = (0, 0);
        if let Ok(mut error) = self.input_error.lock() {
            *error = None;
        }
        match self.create(shell, directory, cx) {
            Ok((view, process)) => {
                self.status = format!(
                    "Running · PID {}",
                    process
                        .pid
                        .map_or_else(|| "—".into(), |pid| pid.to_string())
                );
                self.view = Some(view);
                self.process = Some(process);
                self.running = true;
            }
            Err(error) => self.status = format!("Could not start shell: {error}"),
        }
    }

    fn create(
        &self,
        shell: &str,
        directory: &Path,
        cx: &mut App,
    ) -> Result<(Entity<TerminalView>, PtyProcess), String> {
        let mut config = TerminalConfig {
            update_window_title: false,
            ..Default::default()
        };
        config.default_bg.r = (ui::TERMINAL >> 16) as u8;
        config.default_bg.g = (ui::TERMINAL >> 8) as u8;
        config.default_bg.b = ui::TERMINAL as u8;
        config.default_fg.r = (ui::TEXT >> 16) as u8;
        config.default_fg.g = (ui::TEXT >> 8) as u8;
        config.default_fg.b = ui::TEXT as u8;
        let terminal =
            TerminalSession::new(config).map_err(|error| format!("Ghostty VT: {error:?}"))?;
        let process = PtyProcess::spawn(
            shell,
            &["-l"],
            directory,
            PtySize {
                rows: config.rows,
                cols: config.cols,
                pixel_width: 0,
                pixel_height: 0,
            },
        )?;
        let sender = process.input_sender()?;
        let input_error = self.input_error.clone();
        let input_epoch = self.input_epoch.clone();
        let input = TerminalInput::new(move |bytes| {
            // Conservatively retain companions after any input, including VT replies.
            input_epoch.fetch_add(1, Ordering::Relaxed);
            let result = if bytes.len() > 4 * 1024 * 1024 {
                Err("Paste exceeds this app's 4 MiB limit".to_string())
            } else {
                sender
                    .try_send(bytes.to_vec())
                    .map_err(|error| match error {
                        mpsc::TrySendError::Full(_) => "Input queue is full; wait and retry".into(),
                        mpsc::TrySendError::Disconnected(_) => {
                            "Session has exited; restart to open another shell".into()
                        }
                    })
            };
            if let Err(message) = result {
                eprintln!("{message}");
                if let Ok(mut pending) = input_error.lock() {
                    *pending = Some(message);
                }
            }
        });
        let focus = self.focus.clone();
        let font = self.font.clone();
        let view = cx.new(move |view_cx| {
            let mut view = TerminalView::new_with_input(terminal, focus, input);
            view.set_font(font, view_cx);
            view.set_copy_on_select(true);
            view.on_copied(|window, cx| crate::toast::show("已复制", window, cx));
            view
        });
        Ok((view, process))
    }

    pub fn retire(mut self) {
        self.view.take();
        if let Some(process) = self.process.take() {
            // Waiting for SIGHUP and reaping a child must not stall the UI thread.
            if let Err(error) = std::thread::Builder::new()
                .name("terminal-retire".into())
                .spawn(move || drop(process))
            {
                eprintln!("Start terminal cleanup: {error}");
            }
        }
    }

    pub fn input_epoch(&self) -> u64 {
        self.input_epoch.load(Ordering::Relaxed)
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    pub fn poll(&mut self, cx: &mut App) -> bool {
        let mut batch = Vec::new();
        let mut status = None;
        if let Some(process) = &mut self.process {
            while batch.len() < 256 * 1024 {
                match process.output.try_recv() {
                    Ok(Output::Bytes(bytes)) => batch.extend_from_slice(&bytes),
                    Ok(Output::Error(error)) => status = Some(error),
                    Ok(Output::Eof) => break,
                    Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => break,
                }
            }
            match process.poll_exit() {
                Ok(Some(exit)) => {
                    self.running = false;
                    status = Some(exit);
                }
                Ok(None) => {}
                Err(error) => status = Some(format!("Process error: {error}")),
            }
        }
        if !batch.is_empty()
            && let Some(view) = &self.view
        {
            view.update(cx, |view, cx| view.queue_output_bytes(&batch, cx));
        }
        if let Ok(mut pending) = self.input_error.lock()
            && let Some(error) = pending.take()
        {
            status = Some(error);
        }
        if let Some(status) = status
            && status != self.status
        {
            if self.running {
                self.notice = Some(status.clone());
            }
            self.status = status;
            return true;
        }
        false
    }

    pub fn resize(&mut self, cols: u16, rows: u16, pixel_size: (u16, u16), cx: &mut App) -> bool {
        if self.grid == (cols, rows) && self.pixel_size == pixel_size {
            return false;
        }
        let grid_changed = self.grid != (cols, rows);
        self.grid = (cols, rows);
        self.pixel_size = pixel_size;
        if let Some(process) = &self.process
            && let Err(error) = process.resize(PtySize {
                rows,
                cols,
                pixel_width: pixel_size.0,
                pixel_height: pixel_size.1,
            })
        {
            self.status = format!("Resize failed: {error}");
            self.notice = Some(self.status.clone());
        }
        if grid_changed && let Some(view) = &self.view {
            view.update(cx, |view, cx| view.resize_terminal(cols, rows, cx));
        }
        true
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.view.take();
        self.process.take();
    }
}
