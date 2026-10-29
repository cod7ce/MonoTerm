use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    path::Path,
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};

pub enum Output {
    Bytes(Vec<u8>),
    Error(String),
    Eof,
}

pub struct PtyProcess {
    master: Option<Box<dyn MasterPty + Send>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    input: Option<SyncSender<Vec<u8>>>,
    pub output: Receiver<Output>,
    pub pid: Option<u32>,
}

impl PtyProcess {
    pub fn spawn(
        shell: &str,
        arguments: &[&str],
        directory: &Path,
        size: PtySize,
    ) -> Result<Self, String> {
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| e.to_string())?;
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let mut command = CommandBuilder::new(shell);
        command.args(arguments);
        command.cwd(directory);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "MonoTerm");
        command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        // A fresh terminal must not inherit the launcher’s multiplexer or agent session.
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("HERDR_")
                || name.starts_with("CLAUDE_CODE_")
                || matches!(
                    name.as_ref(),
                    "TMUX" | "TMUX_PANE" | "CLAUDECODE" | "CLAUDE_PID"
                )
            {
                command.env_remove(key);
            }
        }
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| e.to_string())?;
        drop(pair.slave);
        let pid = child.process_id();
        let (output_tx, output) = mpsc::sync_channel(64);
        let (input_tx, input_rx) = mpsc::sync_channel::<Vec<u8>>(64);
        let process = Self {
            master: Some(pair.master),
            child: Some(child),
            input: Some(input_tx),
            output,
            pid,
        };
        let reader_tx = output_tx.clone();
        thread::Builder::new()
            .name("terminal-read".into())
            .spawn(move || {
                let mut buffer = [0_u8; 16 * 1024];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(length) => {
                            if reader_tx
                                .send(Output::Bytes(buffer[..length].to_vec()))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        // Unix PTYs may report EIO when the slave has closed.
                        Err(error) if error.raw_os_error() == Some(5) => break,
                        Err(error) => {
                            if reader_tx
                                .send(Output::Error(format!("PTY read: {error}")))
                                .is_err()
                            {
                                return;
                            }
                            break;
                        }
                    }
                }
                let _ = reader_tx.send(Output::Eof);
            })
            .map_err(|e| format!("Start PTY reader: {e}"))?;
        thread::Builder::new()
            .name("terminal-write".into())
            .spawn(move || {
                while let Ok(bytes) = input_rx.recv() {
                    if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                        let _ = output_tx.send(Output::Error(format!("PTY write: {error}")));
                        break;
                    }
                }
            })
            .map_err(|e| format!("Start PTY writer: {e}"))?;
        Ok(process)
    }

    pub fn input_sender(&self) -> Result<SyncSender<Vec<u8>>, String> {
        self.input.clone().ok_or_else(|| "Session is closed".into())
    }

    #[cfg(test)]
    pub fn send(&self, bytes: &[u8]) -> Result<(), String> {
        let input = self.input.as_ref().ok_or("Session is closed")?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("Paste exceeds this prototype's 4 MiB limit".into());
        }
        input.try_send(bytes.to_vec()).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => {
                "Input queue is full; wait for the process and retry".into()
            }
            mpsc::TrySendError::Disconnected(_) => "Session input is closed".into(),
        })
    }

    pub fn foreground_pgid(&self) -> Option<i32> {
        self.master.as_ref()?.process_group_leader()
    }

    pub fn resize(&self, size: PtySize) -> Result<(), String> {
        self.master
            .as_ref()
            .ok_or("Session is closed")?
            .resize(size)
            .map_err(|e| e.to_string())
    }

    pub fn poll_exit(&mut self) -> Result<Option<String>, String> {
        let Some(child) = &mut self.child else {
            return Ok(None);
        };
        let status = child.try_wait().map_err(|e| e.to_string())?;
        if let Some(status) = status {
            self.child.take();
            self.input.take();
            return Ok(Some(format!("Exited ({})", status.exit_code())));
        }
        Ok(None)
    }

    pub fn shutdown(&mut self) {
        self.input.take();
        self.master.take();
        if let Some(mut child) = self.child.take() {
            // portable-pty sends SIGHUP first, with a bounded grace period before killing.
            if let Err(error) = child.kill() {
                eprintln!("Stop shell: {error}");
            }
            if let Err(error) = child.wait() {
                eprintln!("Reap shell: {error}");
            }
        }
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn real_pty_accepts_input_and_propagates_resize() {
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };
        let mut process = PtyProcess::spawn(
            "/bin/sh",
            &[
                "-c",
                "read value; printf 'RESULT:%s\\n' \"$value\"; stty size; printf 'CONTEXT:%s:%s:%s\\n' \"${HERDR_ENV-unset}\" \"${TMUX-unset}\" \"${CLAUDECODE-unset}\"",
            ],
            Path::new("/tmp"),
            size,
        )
        .expect("spawn PTY");
        process
            .resize(PtySize {
                rows: 37,
                cols: 101,
                ..size
            })
            .expect("resize");
        process
            .send("你好-terminal\n".as_bytes())
            .expect("send input");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut bytes = Vec::new();
        while Instant::now() < deadline {
            match process.output.recv_timeout(Duration::from_millis(100)) {
                Ok(Output::Bytes(chunk)) => bytes.extend(chunk),
                Ok(Output::Eof) => break,
                Ok(Output::Error(error)) => panic!("{error}"),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let output = String::from_utf8_lossy(&bytes);
        assert!(output.contains("RESULT:你好-terminal"), "{output}");
        assert!(output.contains("37 101"), "{output}");
        assert!(output.contains("CONTEXT:unset:unset:unset"), "{output}");
        process.shutdown();
    }

    #[test]
    fn shutdown_reaps_an_active_shell() {
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };
        let mut process =
            PtyProcess::spawn("/bin/sh", &["-c", "read value"], Path::new("/tmp"), size)
                .expect("spawn PTY");
        let started = Instant::now();
        process.shutdown();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(process.child.is_none());
        assert!(process.send(b"x").is_err());
    }
}
