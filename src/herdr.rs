//! Read-only Herdr protocol 22 client. Socket and native process work stays off
//! the UI thread; events invalidate snapshots and never establish process death.

use crate::process_monitor::{self, ExitWatch, ProcessIdentity};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PROTOCOL: u32 = 22;
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const IO_POLL: Duration = Duration::from_millis(50);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const RECONCILE_INTERVAL: Duration = Duration::from_secs(2);
const QUIET_INTERVAL: Duration = Duration::from_millis(100);
const MAX_COALESCE: Duration = Duration::from_millis(250);
const EXIT_GRACE: Duration = Duration::from_secs(1);
const SUBSCRIPTIONS: &[&str] = &[
    "workspace.focused",
    "workspace.updated",
    "workspace.moved",
    "workspace.closed",
    "tab.focused",
    "tab.created",
    "tab.moved",
    "tab.closed",
    "pane.focused",
    "pane.created",
    "pane.updated",
    "pane.agent_detected",
    "pane.moved",
    "pane.closed",
    "pane.exited",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct NativeSessionRef {
    pub source: String,
    pub agent: String,
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentTarget {
    pub source: String,
    pub terminal_id: String,
    pub pane_id: String,
    pub agent: String,
    pub title: Option<String>,
    pub native_session: Option<NativeSessionRef>,
    pub identity: ProcessIdentity,
    pub directory: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub epoch: u64,
    pub focused_pane_id: Option<String>,
    pub agents: Vec<AgentTarget>,
    pub confirmed_dead: Vec<ProcessIdentity>,
    pub diagnostic: Option<String>,
}

#[derive(Debug)]
pub enum Update {
    Synchronized(Snapshot),
    Disconnected(String),
}

pub struct Client {
    mailbox: Arc<Mutex<Option<Update>>>,
    stop: Arc<AtomicBool>,
    worker: thread::Thread,
}

impl Client {
    pub fn start(socket_path: String) -> Result<Self, String> {
        if socket_path.is_empty() {
            return Err("Herdr socket path is empty".into());
        }
        let mailbox = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_mailbox = mailbox.clone();
        let worker_stop = stop.clone();
        let handle = thread::Builder::new()
            .name("herdr-observer".into())
            .spawn(move || worker(socket_path, worker_mailbox, worker_stop))
            .map_err(|error| format!("Start Herdr observer: {error}"))?;
        // Detach: Drop signals cancellation, but never joins on the UI thread.
        Ok(Self {
            mailbox,
            stop,
            worker: handle.thread().clone(),
        })
    }

    pub fn take_update(&self) -> Option<Update> {
        self.mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.unpark();
    }
}

fn publish(mailbox: &Mutex<Option<Update>>, update: Update) {
    *mailbox.lock().unwrap_or_else(|e| e.into_inner()) = Some(update);
}

fn cancelled(stop: &AtomicBool) -> Result<(), String> {
    if stop.load(Ordering::Acquire) {
        Err("Herdr observer stopped".into())
    } else {
        Ok(())
    }
}

/// Retains partial frames across read timeouts and unread coalesced frames.
/// fill_buf/consume prevents allocation beyond the per-frame limit.
struct Connection {
    reader: BufReader<UnixStream>,
    partial: Vec<u8>,
}

impl Connection {
    fn connect(socket_path: &str) -> Result<Self, String> {
        let stream = UnixStream::connect(socket_path)
            .map_err(|error| format!("Connect to Herdr: {error}"))?;
        stream
            .set_read_timeout(Some(IO_POLL))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_millis(250)))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            reader: BufReader::new(stream),
            partial: Vec::new(),
        })
    }

    fn send(&mut self, id: &str, method: &str, params: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&json!({"id": id, "method": method, "params": params}))
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err("Herdr request exceeds 4 MiB".into());
        }
        bytes.push(b'\n');
        self.reader
            .get_mut()
            .write_all(&bytes)
            .map_err(|e| format!("Write Herdr request: {e}"))
    }

    fn next(&mut self, stop: &AtomicBool, deadline: Instant) -> Result<Option<Value>, String> {
        loop {
            cancelled(stop)?;
            if Instant::now() >= deadline {
                return Ok(None);
            }
            let available = match self.reader.fill_buf() {
                Ok([]) => {
                    return Err(if self.partial.is_empty() {
                        "Herdr socket closed".into()
                    } else {
                        "Herdr socket closed during a frame".into()
                    });
                }
                Ok(bytes) => bytes,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(error) => return Err(format!("Read Herdr frame: {error}")),
            };
            let end = available.iter().position(|byte| *byte == b'\n');
            let count = end.unwrap_or(available.len());
            if self.partial.len() + count > MAX_FRAME_BYTES {
                return Err("Herdr frame exceeds 4 MiB".into());
            }
            self.partial.extend_from_slice(&available[..count]);
            self.reader.consume(count + usize::from(end.is_some()));
            if end.is_some() {
                let value = serde_json::from_slice(&self.partial)
                    .map_err(|e| format!("Invalid Herdr JSON frame: {e}"));
                self.partial.clear();
                return value.map(Some);
            }
        }
    }

    fn reply(&mut self, stop: &AtomicBool, id: &str, result_type: &str) -> Result<Value, String> {
        let frame = self
            .next(stop, Instant::now() + REQUEST_TIMEOUT)?
            .ok_or_else(|| "Herdr request timed out".to_string())?;
        if frame.get("id").and_then(Value::as_str) != Some(id) {
            return Err("Herdr response has an unexpected request ID".into());
        }
        if let Some(error) = frame.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            return Err(format!("Herdr {code}: {message}"));
        }
        let result = frame
            .get("result")
            .ok_or_else(|| "Herdr response has no result".to_string())?;
        if result.get("type").and_then(Value::as_str) != Some(result_type) {
            return Err(format!("Herdr response is not {result_type}"));
        }
        Ok(result.clone())
    }
}

fn request(
    socket_path: &str,
    stop: &AtomicBool,
    method: &str,
    params: Value,
    result_type: &str,
) -> Result<Value, String> {
    cancelled(stop)?;
    let mut connection = Connection::connect(socket_path)?;
    connection.send(method, method, params)?;
    connection.reply(stop, method, result_type)
}

fn subscribe(socket_path: &str, stop: &AtomicBool) -> Result<Connection, String> {
    cancelled(stop)?;
    let mut connection = Connection::connect(socket_path)?;
    let subscriptions: Vec<_> = SUBSCRIPTIONS
        .iter()
        .map(|kind| json!({"type": kind}))
        .collect();
    connection.send(
        "events.subscribe",
        "events.subscribe",
        json!({"subscriptions": subscriptions}),
    )?;
    connection.reply(stop, "events.subscribe", "subscription_started")?;
    Ok(connection)
}

#[derive(Debug, Deserialize)]
struct WireSnapshot {
    protocol: u32,
    focused_pane_id: Option<String>,
    panes: Vec<WirePane>,
    #[serde(default)]
    workspaces: Vec<WireWorkspace>,
    #[serde(default)]
    tabs: Vec<WireTab>,
}

#[derive(Debug, Deserialize)]
struct WireWorkspace {
    workspace_id: String,
    label: String,
}

#[derive(Debug, Deserialize)]
struct WireTab {
    workspace_id: String,
    tab_id: String,
    label: String,
}

#[derive(Debug, Deserialize)]
struct WirePane {
    pane_id: String,
    terminal_id: String,
    workspace_id: Option<String>,
    tab_id: Option<String>,
    agent: Option<String>,
    agent_session: Option<NativeSessionRef>,
    #[serde(skip)]
    companion_title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProcessInfo {
    pane_id: String,
    shell_pid: Option<u32>,
    #[serde(default)]
    foreground_processes: Vec<ForegroundProcess>,
}

#[derive(Debug, Deserialize)]
struct ForegroundProcess {
    pid: u32,
    name: String,
    argv0: Option<String>,
}

fn parse_snapshot(result: Value) -> Result<WireSnapshot, String> {
    let mut snapshot: WireSnapshot = serde_json::from_value(
        result
            .get("snapshot")
            .cloned()
            .ok_or_else(|| "Herdr response has no snapshot".to_string())?,
    )
    .map_err(|e| format!("Invalid Herdr snapshot: {e}"))?;
    if snapshot.protocol != PROTOCOL {
        return Err(format!(
            "Unsupported Herdr protocol {}; expected {PROTOCOL}",
            snapshot.protocol
        ));
    }
    let mut panes = HashSet::new();
    let mut terminals = HashSet::new();
    for pane in &snapshot.panes {
        if pane.pane_id.is_empty()
            || pane.terminal_id.is_empty()
            || !panes.insert(&pane.pane_id)
            || !terminals.insert(&pane.terminal_id)
        {
            return Err("Herdr snapshot has empty or duplicate pane/terminal IDs".into());
        }
    }
    for pane in &mut snapshot.panes {
        let workspace = snapshot.workspaces.iter().find(|workspace| {
            Some(workspace.workspace_id.as_str()) == pane.workspace_id.as_deref()
        });
        let tab = snapshot.tabs.iter().find(|tab| {
            Some(tab.tab_id.as_str()) == pane.tab_id.as_deref()
                && Some(tab.workspace_id.as_str()) == pane.workspace_id.as_deref()
        });
        let parts: Vec<_> = [
            workspace.map(|workspace| workspace.label.as_str()),
            tab.map(|tab| tab.label.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter_map(|label| crate::layout::TabAppearance::new(label, None).title)
        .collect();
        pane.companion_title = (!parts.is_empty()).then(|| parts.join(" · "));
    }
    Ok(snapshot)
}

fn native_compatible(old: &Option<NativeSessionRef>, new: &Option<NativeSessionRef>) -> bool {
    match (old, new) {
        (Some(old), Some(new)) => old == new,
        _ => true,
    }
}

fn matches_agent(process: &ForegroundProcess, agent: &str) -> bool {
    [Some(process.name.as_str()), process.argv0.as_deref()]
        .into_iter()
        .flatten()
        .any(|name| Path::new(name).file_name().and_then(|name| name.to_str()) == Some(agent))
}

fn native_directory(identity: &ProcessIdentity) -> Result<PathBuf, String> {
    let directory = process_monitor::process_directory(identity)?;
    if !directory.is_absolute() || !directory.is_dir() {
        return Err("Agent directory is not an existing absolute directory".into());
    }
    Ok(directory)
}

fn resolve_target(
    source: &str,
    pane: &WirePane,
    info: &ProcessInfo,
    cached: Option<&AgentTarget>,
) -> Result<Option<AgentTarget>, String> {
    if info.pane_id != pane.pane_id {
        return Err("Herdr process response has an unexpected pane ID".into());
    }
    let agent = pane
        .agent
        .as_deref()
        .or_else(|| cached.map(|target| target.agent.as_str()));
    let Some(agent @ ("claude" | "codex")) = agent else {
        return Ok(None);
    };
    if pane.agent_session.as_ref().is_some_and(|native| {
        native.agent != agent
            || native.source.is_empty()
            || native.value.is_empty()
            || !matches!(native.kind.as_str(), "id" | "path")
    }) {
        return Err("Herdr native session identity is incompatible with the agent".into());
    }
    let candidates: Vec<_> = info
        .foreground_processes
        .iter()
        .filter(|process| matches_agent(process, agent))
        .collect();
    let identity = if candidates.is_empty() {
        let Some(cached) = cached.filter(|cached| {
            cached.terminal_id == pane.terminal_id
                && cached.agent == agent
                && native_compatible(&cached.native_session, &pane.agent_session)
        }) else {
            return Ok(None);
        };
        // SIGSTOP, backgrounding, and detection loss are not process exits.
        if !process_monitor::is_alive(&cached.identity)? {
            return Ok(None);
        }
        cached.identity.clone()
    } else {
        let shell_pid = info
            .shell_pid
            .filter(|pid| *pid > 0)
            .ok_or_else(|| "Herdr has no local shell PID for this agent".to_string())?;
        let mut identities = HashSet::new();
        for candidate in candidates {
            if let Ok(identity) = process_monitor::identify_agent(shell_pid, candidate.pid, agent) {
                identities.insert(identity);
            }
        }
        if identities.len() != 1 {
            return Err(format!(
                "Expected one verified local {agent} process; found {}",
                identities.len()
            ));
        }
        identities.into_iter().next().unwrap()
    };
    let directory = native_directory(&identity)?;
    // Retain a known native reference through transient missing metadata, but
    // never carry it onto a newly observed process incarnation.
    let native_session = pane.agent_session.clone().or_else(|| {
        cached
            .filter(|cached| cached.identity == identity && cached.agent == agent)
            .and_then(|cached| cached.native_session.clone())
    });
    Ok(Some(AgentTarget {
        source: source.into(),
        terminal_id: pane.terminal_id.clone(),
        pane_id: pane.pane_id.clone(),
        agent: agent.into(),
        title: pane.companion_title.clone(),
        native_session,
        identity,
        directory,
    }))
}

struct TrackedProcess {
    watch: Option<ExitWatch>,
    dead_since: Option<Instant>,
}

#[derive(Default)]
struct State {
    epoch: u64,
    cached: HashMap<String, AgentTarget>,
    tracked: HashMap<ProcessIdentity, TrackedProcess>,
    // Cumulative for the client's lifetime: replacing a mailbox snapshot must
    // not lose a process-exit confirmation that the UI has not consumed yet.
    confirmed_dead: HashSet<ProcessIdentity>,
    process_diagnostic: Option<String>,
}

impl State {
    fn observe_exits(&mut self) {
        for (identity, tracked) in &mut self.tracked {
            if tracked.dead_since.is_some() {
                continue;
            }
            let observed = match tracked.watch.as_mut() {
                Some(watch) => watch.exited(),
                None => process_monitor::is_alive(identity).map(|alive| !alive),
            };
            match observed {
                Ok(true) => tracked.dead_since = Some(Instant::now()),
                Ok(false) => {}
                Err(error) => {
                    // A watcher error is unknown. A separate native identity
                    // check may still positively establish that it has exited.
                    if matches!(process_monitor::is_alive(identity), Ok(false)) {
                        tracked.dead_since = Some(Instant::now());
                    } else {
                        self.process_diagnostic = Some(error);
                    }
                }
            }
        }
    }

    fn confirm_exits(&mut self, snapshot_started: Instant) {
        self.tracked.retain(|identity, tracked| {
            let confirmed = tracked.dead_since.is_some_and(|since| {
                snapshot_started.saturating_duration_since(since) >= EXIT_GRACE
            });
            if confirmed {
                self.confirmed_dead.insert(identity.clone());
            }
            !confirmed
        });
    }

    fn reconcile(&mut self, source: &str, stop: &AtomicBool) -> Result<Snapshot, String> {
        self.observe_exits();
        let snapshot_started = Instant::now();
        let wire = parse_snapshot(request(
            source,
            stop,
            "session.snapshot",
            json!({}),
            "session_snapshot",
        )?)?;
        // Keep the last verified association through transient inspection errors.
        // It is not emitted unless this reconciliation validates it again.
        self.cached.retain(|terminal_id, cached| {
            wire.panes.iter().any(|pane| {
                pane.terminal_id == *terminal_id
                    && pane
                        .agent
                        .as_deref()
                        .is_none_or(|agent| agent == cached.agent)
                    && native_compatible(&cached.native_session, &pane.agent_session)
            })
        });
        let mut agents = Vec::new();
        let mut diagnostic = self.process_diagnostic.take();
        for pane in wire.panes {
            cancelled(stop)?;
            let cached = self.cached.get(&pane.terminal_id);
            if !pane
                .agent
                .as_deref()
                .is_some_and(|kind| matches!(kind, "claude" | "codex"))
                && cached.is_none()
            {
                continue;
            }
            let resolved = (|| {
                let result = request(
                    source,
                    stop,
                    "pane.process_info",
                    json!({"pane_id": pane.pane_id}),
                    "pane_process_info",
                )?;
                let info: ProcessInfo = serde_json::from_value(
                    result
                        .get("process_info")
                        .cloned()
                        .ok_or_else(|| "Herdr response has no process_info".to_string())?,
                )
                .map_err(|e| format!("Invalid Herdr process info: {e}"))?;
                resolve_target(source, &pane, &info, cached)
            })();
            match resolved {
                Ok(Some(target)) if !self.confirmed_dead.contains(&target.identity) => {
                    self.tracked
                        .entry(target.identity.clone())
                        .or_insert_with(|| {
                            let watch = match ExitWatch::new(target.identity.clone()) {
                                Ok(watch) => Some(watch),
                                Err(error) => {
                                    diagnostic.get_or_insert(error);
                                    None
                                }
                            };
                            TrackedProcess {
                                watch,
                                dead_since: None,
                            }
                        });
                    agents.push(target);
                }
                Ok(_) => {}
                Err(error) => {
                    diagnostic.get_or_insert(format!("{}: {error}", pane.pane_id));
                }
            }
        }
        cancelled(stop)?;
        self.observe_exits();
        // Only a successful fresh snapshot acquired after the grace period can
        // release a positive exit observation to the controller.
        self.confirm_exits(snapshot_started);
        agents.retain(|target| !self.confirmed_dead.contains(&target.identity));
        self.cached
            .retain(|_, target| !self.confirmed_dead.contains(&target.identity));
        self.cached.extend(
            agents
                .iter()
                .map(|target| (target.terminal_id.clone(), target.clone())),
        );
        let mut confirmed_dead: Vec<_> = self.confirmed_dead.iter().cloned().collect();
        confirmed_dead.sort_by_key(|id| (id.pid, id.start_time_secs, id.start_time_micros));
        Ok(Snapshot {
            epoch: self.epoch,
            focused_pane_id: wire.focused_pane_id,
            agents,
            confirmed_dead,
            diagnostic,
        })
    }
}

fn serve_connection(
    source: &str,
    mailbox: &Mutex<Option<Update>>,
    stop: &AtomicBool,
    state: &mut State,
    synchronized: &mut bool,
) -> Result<(), String> {
    // Subscribe first so events occurring during the snapshot remain queued on
    // this connection. Every request uses a separate, short-lived connection.
    let mut events = subscribe(source, stop)?;
    state.epoch = state.epoch.saturating_add(1);
    let mut dirty: Option<(Instant, Instant)> = None;
    let mut next_reconcile = Instant::now();
    loop {
        cancelled(stop)?;
        state.observe_exits();
        let now = Instant::now();
        let invalidated = dirty.is_some_and(|(first, last)| {
            now.duration_since(last) >= QUIET_INTERVAL || now.duration_since(first) >= MAX_COALESCE
        });
        if now >= next_reconcile || invalidated {
            let snapshot = state.reconcile(source, stop)?;
            publish(mailbox, Update::Synchronized(snapshot));
            *synchronized = true;
            next_reconcile = Instant::now() + RECONCILE_INTERVAL;
            dirty = None;
        }
        if let Some(event) = events.next(stop, Instant::now() + IO_POLL)? {
            if event.get("event").and_then(Value::as_str).is_none()
                || !event.get("data").is_some_and(Value::is_object)
            {
                return Err("Invalid Herdr subscription event".into());
            }
            let now = Instant::now();
            match &mut dirty {
                Some((_, last)) => *last = now,
                None => dirty = Some((now, now)),
            }
        }
    }
}

fn worker(source: String, mailbox: Arc<Mutex<Option<Update>>>, stop: Arc<AtomicBool>) {
    let mut state = State::default();
    let backoff = [500, 1_000, 2_000, 4_000, 10_000];
    let mut attempt = 0;
    while !stop.load(Ordering::Acquire) {
        let mut synchronized = false;
        let result = serve_connection(&source, &mailbox, &stop, &mut state, &mut synchronized);
        if stop.load(Ordering::Acquire) {
            break;
        }
        if let Err(error) = result {
            publish(&mailbox, Update::Disconnected(error));
        }
        if synchronized {
            attempt = 0;
        }
        let deadline = Instant::now() + Duration::from_millis(backoff[attempt]);
        attempt = (attempt + 1).min(backoff.len() - 1);
        while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
            state.observe_exits();
            thread::park_timeout(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(IO_POLL),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::AtomicU64;

    struct MockSocket(PathBuf);

    impl MockSocket {
        fn start(
            handler: impl FnOnce(UnixStream) + Send + 'static,
        ) -> (Self, thread::JoinHandle<()>) {
            Self::serve(move |listener| handler(listener.accept().unwrap().0))
        }

        fn serve(
            handler: impl FnOnce(UnixListener) + Send + 'static,
        ) -> (Self, thread::JoinHandle<()>) {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = PathBuf::from(format!(
                "/tmp/monoterm-herdr-{}-{}.sock",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let listener = UnixListener::bind(&path).unwrap();
            let handle = thread::spawn(move || handler(listener));
            (Self(path), handle)
        }

        fn path(&self) -> &str {
            self.0.to_str().unwrap()
        }
    }

    impl Drop for MockSocket {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn read_request(stream: &mut UnixStream) -> Value {
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn split_frames_survive_timeout_and_coalesced_frames_remain_buffered() {
        let (socket, server) = MockSocket::start(|mut stream| {
            stream.write_all(b"{\"n\":").unwrap();
            thread::sleep(IO_POLL * 3);
            stream.write_all(b"1}\n{\"n\":2}\n").unwrap();
        });
        let mut connection = Connection::connect(socket.path()).unwrap();
        let stop = AtomicBool::new(false);
        assert!(
            connection
                .next(&stop, Instant::now() + IO_POLL)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            connection
                .next(&stop, Instant::now() + REQUEST_TIMEOUT)
                .unwrap(),
            Some(json!({"n":1}))
        );
        assert_eq!(
            connection
                .next(&stop, Instant::now() + REQUEST_TIMEOUT)
                .unwrap(),
            Some(json!({"n":2}))
        );
        assert!(
            connection
                .next(&stop, Instant::now() + REQUEST_TIMEOUT)
                .unwrap_err()
                .contains("closed")
        );
        server.join().unwrap();
    }

    #[test]
    fn eof_in_partial_frame_is_an_error() {
        let (socket, server) = MockSocket::start(|mut stream| {
            stream.write_all(b"{\"result\":").unwrap();
        });
        let mut connection = Connection::connect(socket.path()).unwrap();
        assert!(
            connection
                .next(&AtomicBool::new(false), Instant::now() + REQUEST_TIMEOUT)
                .unwrap_err()
                .contains("during a frame")
        );
        server.join().unwrap();
    }

    #[test]
    fn frame_limit_is_enforced_without_waiting_for_newline() {
        let (socket, server) = MockSocket::start(|mut stream| {
            let _ = stream.write_all(&vec![b' '; MAX_FRAME_BYTES + 1]);
        });
        let mut connection = Connection::connect(socket.path()).unwrap();
        assert!(
            connection
                .next(&AtomicBool::new(false), Instant::now() + REQUEST_TIMEOUT)
                .unwrap_err()
                .contains("exceeds 4 MiB")
        );
        assert!(connection.partial.len() <= MAX_FRAME_BYTES);
        server.join().unwrap();
    }

    #[test]
    fn subscription_uses_global_filters_and_preserves_coalesced_event() {
        let (socket, server) = MockSocket::start(|mut stream| {
            let request = read_request(&mut stream);
            assert_eq!(request["method"], "events.subscribe");
            let subscriptions = request["params"]["subscriptions"].as_array().unwrap();
            assert!(subscriptions.contains(&json!({"type":"pane.agent_detected"})));
            assert!(
                subscriptions
                    .iter()
                    .all(|subscription| subscription.get("pane_id").is_none())
            );
            stream.write_all(b"{\"id\":\"events.subscribe\",\"result\":{\"type\":\"subscription_started\"}}\n{\"event\":\"pane_exited\",\"data\":{\"pane_id\":\"w1:p1\"}}\n").unwrap();
        });
        let stop = AtomicBool::new(false);
        let mut connection = subscribe(socket.path(), &stop).unwrap();
        assert_eq!(
            connection
                .next(&stop, Instant::now() + REQUEST_TIMEOUT)
                .unwrap()
                .unwrap()["event"],
            "pane_exited"
        );
        server.join().unwrap();
    }

    #[test]
    fn subscription_rejects_ack_errors_wrong_ids_and_wrong_types() {
        for response in [
            json!({"id":"events.subscribe","error":{"code":"invalid_params","message":"bad filter"}}),
            json!({"id":"wrong","result":{"type":"subscription_started"}}),
            json!({"id":"events.subscribe","result":{"type":"pong"}}),
        ] {
            let (socket, server) = MockSocket::start(move |mut stream| {
                read_request(&mut stream);
                writeln!(stream, "{response}").unwrap();
            });
            assert!(subscribe(socket.path(), &AtomicBool::new(false)).is_err());
            server.join().unwrap();
        }
    }

    fn snapshot_result(protocol: u32) -> Value {
        json!({"type":"session_snapshot","snapshot": {
            "protocol":protocol,"focused_pane_id":"w1:p1",
            "panes":[{"pane_id":"w1:p1","terminal_id":"term_1","agent":"claude",
                "agent_session":{"source":"herdr:claude","agent":"claude","kind":"id","value":"session-1"},
                "cwd":"/untrusted/path","foreground_cwd":"/also/untrusted"}],
            "agents":[],"version":"0.9.5"
        }})
    }

    #[test]
    fn snapshot_request_uses_exact_envelope_and_rejects_other_protocols() {
        for protocol in [22, 21, 23] {
            let (socket, server) = MockSocket::start(move |mut stream| {
                let request = read_request(&mut stream);
                assert_eq!(
                    request,
                    json!({"id":"session.snapshot","method":"session.snapshot","params":{}})
                );
                writeln!(
                    stream,
                    "{}",
                    json!({"id":"session.snapshot","result":snapshot_result(protocol)})
                )
                .unwrap();
            });
            let result = request(
                socket.path(),
                &AtomicBool::new(false),
                "session.snapshot",
                json!({}),
                "session_snapshot",
            )
            .unwrap();
            let parsed = parse_snapshot(result);
            if protocol == 22 {
                let parsed = parsed.unwrap();
                assert_eq!(parsed.focused_pane_id.as_deref(), Some("w1:p1"));
                assert_eq!(parsed.panes[0].terminal_id, "term_1");
                assert_eq!(
                    parsed.panes[0].agent_session.as_ref().unwrap().value,
                    "session-1"
                );
            } else {
                assert!(parsed.unwrap_err().contains("Unsupported Herdr protocol"));
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn companion_titles_use_the_owning_space_and_tab_labels() {
        let mut result = snapshot_result(22);
        result["snapshot"]["panes"][0]["workspace_id"] = json!("w1");
        result["snapshot"]["panes"][0]["tab_id"] = json!("w1:t1");
        result["snapshot"]["workspaces"] = json!([
            {"workspace_id":"w2","label":"Other space"},
            {"workspace_id":"w1","label":" 👨‍💻 AboutME "}
        ]);
        result["snapshot"]["tabs"] = json!([
            {"workspace_id":"w2","tab_id":"w1:t1","label":"Wrong owner"},
            {"workspace_id":"w1","tab_id":"w1:t2","label":"Wrong tab"},
            {"workspace_id":"w1","tab_id":"w1:t1","label":"MonoTerm\n"}
        ]);
        result["snapshot"]["agents"] = json!([
            {"pane_id":"w1:p1","name":"not-the-tab-title","agent":"claude"}
        ]);
        let title = |value| {
            parse_snapshot(value)
                .unwrap()
                .panes
                .remove(0)
                .companion_title
        };
        assert_eq!(
            title(result.clone()).as_deref(),
            Some("👨‍💻 AboutME · MonoTerm")
        );
        result["snapshot"]["workspaces"][1]["label"] = json!("🐸 查修");
        result["snapshot"]["tabs"][2]["label"] = json!("集成测试");
        assert_eq!(title(result.clone()).as_deref(), Some("🐸 查修 · 集成测试"));
        result["snapshot"]["tabs"][2]["label"] = json!("\n\t ");
        assert_eq!(title(result.clone()).as_deref(), Some("🐸 查修"));
        result["snapshot"]["workspaces"][1]["label"] = json!("");
        assert_eq!(title(result), None);
        assert_eq!(title(snapshot_result(22)), None);
    }

    #[test]
    fn malformed_or_duplicate_snapshot_inventory_is_rejected() {
        let mut result = snapshot_result(22);
        result["snapshot"]["panes"] = json!([{"pane_id":"w1:p1"}]);
        assert!(parse_snapshot(result).is_err());
        let mut result = snapshot_result(22);
        let pane = result["snapshot"]["panes"][0].clone();
        result["snapshot"]["panes"]
            .as_array_mut()
            .unwrap()
            .push(pane);
        assert!(parse_snapshot(result).is_err());
    }

    #[test]
    fn native_session_compatibility_allows_missing_but_not_conflicting_metadata() {
        let session = NativeSessionRef {
            source: "herdr:claude".into(),
            agent: "claude".into(),
            kind: "id".into(),
            value: "one".into(),
        };
        let mut other = session.clone();
        other.value = "two".into();
        assert!(native_compatible(&Some(session.clone()), &None));
        assert!(native_compatible(&None, &Some(session.clone())));
        assert!(!native_compatible(&Some(session), &Some(other)));
    }

    #[test]
    fn foreground_candidates_include_version_named_claude_after_caffeinate() {
        let info: ProcessInfo = serde_json::from_value(json!({"pane_id":"w1:p1","shell_pid":1,
            "foreground_processes":[{"pid":2,"name":"caffeinate","argv0":"caffeinate"},
                {"pid":3,"name":"2.1.310","argv0":"claude"}]}))
        .unwrap();
        let candidates: Vec<_> = info
            .foreground_processes
            .iter()
            .filter(|p| matches_agent(p, "claude"))
            .collect();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pid, 3);
    }

    #[test]
    fn exits_require_positive_evidence_grace_and_fresh_snapshot_and_remain_cumulative() {
        let identity = ProcessIdentity {
            pid: 42,
            start_time_secs: 1,
            start_time_micros: 2,
        };
        let now = Instant::now();
        let mut state = State::default();
        state.tracked.insert(
            identity.clone(),
            TrackedProcess {
                watch: None,
                dead_since: None,
            },
        );
        state.confirm_exits(now + EXIT_GRACE * 2);
        assert!(state.confirmed_dead.is_empty());
        state.tracked.get_mut(&identity).unwrap().dead_since = Some(now);
        state.confirm_exits(now + EXIT_GRACE / 2);
        assert!(state.confirmed_dead.is_empty());
        state.confirm_exits(now + EXIT_GRACE);
        assert!(state.confirmed_dead.contains(&identity));
        state.confirm_exits(now + EXIT_GRACE * 2);
        assert!(state.confirmed_dead.contains(&identity));
        assert!(state.tracked.is_empty());
    }

    #[test]
    fn client_subscribes_before_snapshot_and_drop_cancels_idle_reader() {
        use std::io::Read;
        let (snapshot_sent, snapshot_received) = std::sync::mpsc::channel();
        let (closed, closed_received) = std::sync::mpsc::channel();
        let (socket, server) = MockSocket::serve(move |listener| {
            let mut events = listener.accept().unwrap().0;
            assert_eq!(read_request(&mut events)["method"], "events.subscribe");
            writeln!(
                events,
                "{}",
                json!({"id":"events.subscribe","result":{"type":"subscription_started"}})
            )
            .unwrap();
            let mut snapshot_socket = listener.accept().unwrap().0;
            assert_eq!(
                read_request(&mut snapshot_socket)["method"],
                "session.snapshot"
            );
            writeln!(
                snapshot_socket,
                "{}",
                json!({"id":"session.snapshot","result":{
                    "type":"session_snapshot","snapshot":{"protocol":22,"panes":[]}
                }})
            )
            .unwrap();
            snapshot_sent.send(()).unwrap();
            events.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
            assert_eq!(events.read(&mut [0u8; 1]).unwrap(), 0);
            closed.send(()).unwrap();
        });
        let client = Client::start(socket.path().into()).unwrap();
        snapshot_received.recv_timeout(REQUEST_TIMEOUT).unwrap();
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            match client.take_update() {
                Some(Update::Synchronized(snapshot)) => {
                    assert_eq!(snapshot.epoch, 1);
                    assert!(snapshot.agents.is_empty());
                    break;
                }
                Some(Update::Disconnected(error)) => panic!("{error}"),
                None => {
                    assert!(Instant::now() < deadline);
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
        let started = Instant::now();
        drop(client);
        assert!(started.elapsed() < Duration::from_millis(100));
        closed_received.recv_timeout(REQUEST_TIMEOUT).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn mailbox_keeps_only_latest_update() {
        let mailbox = Mutex::new(None);
        publish(&mailbox, Update::Disconnected("first".into()));
        publish(&mailbox, Update::Disconnected("second".into()));
        assert!(
            matches!(mailbox.lock().unwrap().take(), Some(Update::Disconnected(message)) if message == "second")
        );
    }
}
