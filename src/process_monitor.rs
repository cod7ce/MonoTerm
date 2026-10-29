//! Local process observations for Follow Herdr. Run these on an I/O worker.
//! An error means unknown, never permission to close a companion terminal.

use std::path::PathBuf;

/// One process incarnation within this boot. Associations are never persisted.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time_secs: u64,
    pub start_time_micros: u64,
}

pub use platform::{ExitWatch, identify_agent, is_alive, process_directory, shell_is_idle};

#[cfg(target_os = "macos")]
mod platform {
    use super::{PathBuf, ProcessIdentity};
    use std::ffi::OsStr;
    use std::io;
    use std::mem::{self, size_of};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    // Darwin sys/proc_info.h; libc 0.2.178 omits these constants.
    const PROC_TTY_ONLY: u32 = 3;
    const PROC_PPID_ONLY: u32 = 6;
    const PROC_FLAG_CONTROLT: u32 = 0x80;
    const MAX_ANCESTORS: usize = 128;
    const MAX_INSPECTION_BYTES: usize = 4 * 1024 * 1024;

    fn native_pid(pid: u32) -> Result<libc::pid_t, String> {
        match i32::try_from(pid) {
            Ok(pid) if pid > 0 => Ok(pid),
            _ => Err("invalid process ID".into()),
        }
    }

    fn os_error(operation: &str, error: io::Error) -> String {
        format!("{operation}: {error}")
    }

    // ESRCH establishes absence. Permission failures, short reads and unexpected
    // results remain unknown; kill(pid, 0) cannot validate a process incarnation.
    fn process_info(pid: u32) -> Result<Option<libc::proc_bsdinfo>, String> {
        let native = native_pid(pid)?;
        let mut info = unsafe { mem::zeroed::<libc::proc_bsdinfo>() };
        let size = size_of::<libc::proc_bsdinfo>();
        let read = unsafe {
            libc::proc_pidinfo(
                native,
                libc::PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast(),
                size as libc::c_int,
            )
        };
        if read <= 0 {
            let error = io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(libc::ESRCH) => Ok(None),
                _ => Err(os_error("read process information", error)),
            };
        }
        if read as usize != size
            || info.pbi_pid != pid
            || info.pbi_start_tvsec == 0
            || info.pbi_start_tvusec >= 1_000_000
        {
            return Err("read process information: incomplete or invalid identity".into());
        }
        Ok(Some(info))
    }

    fn identity_of(info: &libc::proc_bsdinfo) -> ProcessIdentity {
        ProcessIdentity {
            pid: info.pbi_pid,
            start_time_secs: info.pbi_start_tvsec,
            start_time_micros: info.pbi_start_tvusec,
        }
    }

    fn live_info(identity: &ProcessIdentity) -> Result<libc::proc_bsdinfo, String> {
        match process_info(identity.pid)? {
            Some(info) if info.pbi_status != libc::SZOMB && identity_of(&info) == *identity => {
                Ok(info)
            }
            _ => Err("process exited or its PID was reused".into()),
        }
    }

    fn same_user(info: &libc::proc_bsdinfo) -> bool {
        // Reject setuid transitions as well as processes belonging to another user.
        info.pbi_uid == unsafe { libc::geteuid() }
            && info.pbi_ruid == unsafe { libc::getuid() }
            && info.pbi_svuid == info.pbi_uid
    }

    fn session_id(pid: u32) -> Result<libc::pid_t, String> {
        let session = unsafe { libc::getsid(native_pid(pid)?) };
        if session <= 0 {
            Err(os_error("read process session", io::Error::last_os_error()))
        } else {
            Ok(session)
        }
    }

    fn tty_device(info: &libc::proc_bsdinfo) -> Option<u32> {
        // NODEV is (dev_t)-1; zero is not a substitute for a missing TTY.
        (info.pbi_flags & PROC_FLAG_CONTROLT != 0 && info.e_tdev != u32::MAX).then_some(info.e_tdev)
    }

    fn nul_bytes(buffer: &[u8]) -> Result<&[u8], String> {
        let end = buffer
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| "process information contains an unterminated path".to_string())?;
        if end == 0 {
            return Err("process information contains an empty path".into());
        }
        Ok(&buffer[..end])
    }

    fn executable_path(pid: u32) -> Result<PathBuf, String> {
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let read = unsafe {
            libc::proc_pidpath(
                native_pid(pid)?,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
            )
        };
        if read <= 0 {
            return Err(os_error(
                "read process executable",
                io::Error::last_os_error(),
            ));
        }
        if read as usize >= buffer.len() {
            return Err("process executable path was truncated".into());
        }
        let path = PathBuf::from(OsStr::from_bytes(nul_bytes(&buffer)?));
        if !path.is_absolute() {
            return Err("process executable path is not absolute".into());
        }
        Ok(path)
    }

    // KERN_PROCARGS2 contains argc, executable path, padding NULs, then argv.
    // Extract only argv[0]; never log the remaining arguments or environment.
    fn argv0(pid: u32) -> Result<PathBuf, String> {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, native_pid(pid)?];
        let mut size = 0usize;
        let result = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                ptr::null_mut(),
                &mut size,
                ptr::null_mut(),
                0,
            )
        };
        if result != 0 {
            return Err(os_error(
                "read process argument size",
                io::Error::last_os_error(),
            ));
        }
        if !(size_of::<i32>() + 1..=MAX_INSPECTION_BYTES).contains(&size) {
            return Err("process argument buffer exceeds inspection bounds".into());
        }
        let mut bytes = vec![0u8; size];
        let result = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                bytes.as_mut_ptr().cast(),
                &mut size,
                ptr::null_mut(),
                0,
            )
        };
        if result != 0 {
            return Err(os_error(
                "read process arguments",
                io::Error::last_os_error(),
            ));
        }
        if size > bytes.len() || size <= size_of::<i32>() {
            return Err("process argument buffer is incomplete".into());
        }
        bytes.truncate(size);
        let argc = i32::from_ne_bytes(bytes[..4].try_into().unwrap());
        if argc < 1 {
            return Err("process has no argv0".into());
        }
        let executable = nul_bytes(&bytes[4..])?;
        let position = 4 + executable.len();
        let offset = bytes[position..]
            .iter()
            .position(|byte| *byte != 0)
            .ok_or_else(|| "process has no argv0".to_string())?;
        let argument = nul_bytes(&bytes[position + offset..])?;
        Ok(PathBuf::from(OsStr::from_bytes(argument)))
    }

    fn claude_version_path(path: &Path) -> bool {
        // Native Claude installs under .../claude/versions/<version>; proc_pidpath
        // resolves the `claude` symlink and pbi_name can be just the version.
        let Some(version) = path.file_name().and_then(OsStr::to_str) else {
            return false;
        };
        let pieces: Vec<_> = version.split('.').collect();
        let numeric_version = pieces.len() == 3
            && pieces
                .iter()
                .all(|piece| !piece.is_empty() && piece.bytes().all(|byte| byte.is_ascii_digit()));
        let Some(parent) = path.parent() else {
            return false;
        };
        numeric_version
            && parent.file_name() == Some(OsStr::new("versions"))
            && parent.parent().and_then(Path::file_name) == Some(OsStr::new("claude"))
    }

    fn validate_executable(pid: u32, agent_kind: &str) -> Result<PathBuf, String> {
        let executable = executable_path(pid)?;
        if executable.file_name() == Some(OsStr::new(agent_kind)) {
            return Ok(executable);
        }
        if agent_kind == "claude" && claude_version_path(&executable) {
            let argument = argv0(pid)?;
            if argument == executable || argument.file_name() == Some(OsStr::new("claude")) {
                return Ok(executable);
            }
        }
        // argv0 alone is not evidence: `exec -a claude /bin/sleep` must fail.
        // Interpreter-based launchers are intentionally unsupported here.
        Err("process executable does not identify the requested agent".into())
    }

    fn same_relationship(before: &libc::proc_bsdinfo, after: &libc::proc_bsdinfo) -> bool {
        identity_of(before) == identity_of(after)
            && before.pbi_ppid == after.pbi_ppid
            && before.pbi_pgid == after.pbi_pgid
            && tty_device(before) == tty_device(after)
            && same_user(after)
    }

    /// Correlate an untrusted candidate PID with a live local shell. Requires the
    /// current UID, verified ancestry, an executable for the requested agent, and
    /// the same controlling TTY and session. Background/stopped agents are valid.
    pub fn identify_agent(
        shell_pid: u32,
        candidate_pid: u32,
        agent_kind: &str,
    ) -> Result<ProcessIdentity, String> {
        if !matches!(agent_kind, "claude" | "codex") {
            return Err("unsupported agent kind".into());
        }
        if shell_pid == candidate_pid {
            return Err("candidate must be a descendant of the shell".into());
        }
        let shell = process_info(shell_pid)?.ok_or_else(|| "shell is absent".to_string())?;
        if shell.pbi_status == libc::SZOMB || !same_user(&shell) {
            return Err("shell is not live or does not belong to the current user".into());
        }
        let tty = tty_device(&shell)
            .ok_or_else(|| "shell has no verifiable controlling TTY".to_string())?;
        let session = session_id(shell_pid)?;
        let shell_path = shell_executable(shell_pid)?;
        let candidate =
            process_info(candidate_pid)?.ok_or_else(|| "agent is absent".to_string())?;
        let identity = identity_of(&candidate);
        let mut ancestry = vec![candidate];
        loop {
            let current = ancestry.last().unwrap();
            if current.pbi_status == libc::SZOMB
                || !same_user(current)
                || tty_device(current) != Some(tty)
                || session_id(current.pbi_pid)? != session
            {
                return Err(
                    "agent ancestry does not share the shell's user, TTY and session".into(),
                );
            }
            if current.pbi_ppid == shell_pid {
                break;
            }
            if current.pbi_ppid <= 1
                || ancestry.len() >= MAX_ANCESTORS
                || ancestry.iter().any(|info| info.pbi_pid == current.pbi_ppid)
            {
                return Err("agent has no verifiable ancestry to the shell".into());
            }
            let parent = process_info(current.pbi_ppid)?
                .ok_or_else(|| "agent ancestry changed during inspection".to_string())?;
            if (parent.pbi_start_tvsec, parent.pbi_start_tvusec)
                > (current.pbi_start_tvsec, current.pbi_start_tvusec)
            {
                return Err("agent ancestor PID was reused".into());
            }
            ancestry.push(parent);
        }
        let oldest = ancestry.last().unwrap();
        if (shell.pbi_start_tvsec, shell.pbi_start_tvusec)
            > (oldest.pbi_start_tvsec, oldest.pbi_start_tvusec)
        {
            return Err("shell PID was reused".into());
        }
        let executable = validate_executable(candidate_pid, agent_kind)?;
        ancestry.push(shell);
        for before in &ancestry {
            let after = live_info(&identity_of(before))?;
            if !same_relationship(before, &after) || session_id(after.pbi_pid)? != session {
                return Err("agent relationship changed during inspection".into());
            }
        }
        if validate_executable(candidate_pid, agent_kind)? != executable
            || shell_executable(shell_pid)? != shell_path
        {
            return Err("agent or shell executable changed during inspection".into());
        }
        live_info(&identity)?;
        Ok(identity)
    }

    /// Read the native cwd of the same process incarnation. No shell or lsof is
    /// launched. A missing/unreadable/deleted cwd remains an error.
    pub fn process_directory(identity: &ProcessIdentity) -> Result<PathBuf, String> {
        let before = live_info(identity)?;
        if !same_user(&before) {
            return Err("process does not belong to the current user".into());
        }
        let mut info = unsafe { mem::zeroed::<libc::proc_vnodepathinfo>() };
        let size = size_of::<libc::proc_vnodepathinfo>();
        let read = unsafe {
            libc::proc_pidinfo(
                native_pid(identity.pid)?,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&raw mut info).cast(),
                size as libc::c_int,
            )
        };
        if read <= 0 {
            return Err(os_error(
                "read process working directory",
                io::Error::last_os_error(),
            ));
        }
        if read as usize != size {
            return Err("read process working directory: incomplete native result".into());
        }
        // libc represents vip_path as [[c_char; 32]; 32]. Flatten without any
        // UTF-8 conversion or reading beyond the fixed array.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                info.pvi_cdir.vip_path.as_ptr().cast::<u8>(),
                mem::size_of_val(&info.pvi_cdir.vip_path),
            )
        };
        let path = PathBuf::from(OsStr::from_bytes(nul_bytes(bytes)?));
        if !path.is_absolute() || !path.is_dir() {
            return Err("process working directory is not an existing absolute directory".into());
        }
        let after = live_info(identity)?;
        if !same_user(&after) {
            return Err("process user changed during directory inspection".into());
        }
        Ok(path)
    }

    /// False only for absence, PID reuse or a zombie. A stopped process is alive;
    /// permission/inspection errors are unknown and are returned as errors.
    pub fn is_alive(identity: &ProcessIdentity) -> Result<bool, String> {
        Ok(process_info(identity.pid)?
            .is_some_and(|info| identity_of(&info) == *identity && info.pbi_status != libc::SZOMB))
    }

    /// A kqueue registration for one incarnation. OwnedFd closes it on drop.
    pub struct ExitWatch {
        fd: OwnedFd,
        identity: ProcessIdentity,
        observed_exit: bool,
    }

    impl ExitWatch {
        pub fn new(identity: ProcessIdentity) -> Result<Self, String> {
            live_info(&identity)?;
            let raw_fd = unsafe { libc::kqueue() };
            if raw_fd < 0 {
                return Err(os_error("create exit kqueue", io::Error::last_os_error()));
            }
            let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
            if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(os_error(
                    "protect exit kqueue descriptor",
                    io::Error::last_os_error(),
                ));
            }
            let change = libc::kevent {
                ident: identity.pid as libc::uintptr_t,
                filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: ptr::null_mut(),
            };
            let result = unsafe {
                libc::kevent(fd.as_raw_fd(), &change, 1, ptr::null_mut(), 0, ptr::null())
            };
            // Check again even when registration failed (e.g. ESRCH). Never
            // carry a registration over to a different process with this PID.
            let registration_error = (result < 0).then(io::Error::last_os_error);
            live_info(&identity)?;
            if let Some(error) = registration_error {
                return Err(os_error("register process exit watch", error));
            }
            Ok(Self {
                fd,
                identity,
                observed_exit: false,
            })
        }

        /// Nonblocking, with a latched result. Rechecks identity before and after
        /// reading events, including when a process exited between poll calls.
        pub fn exited(&mut self) -> Result<bool, String> {
            if self.observed_exit {
                return Ok(true);
            }
            if !is_alive(&self.identity)? {
                self.observed_exit = true;
                return Ok(true);
            }
            let mut event = unsafe { mem::zeroed::<libc::kevent>() };
            let timeout = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            let count = unsafe {
                libc::kevent(self.fd.as_raw_fd(), ptr::null(), 0, &mut event, 1, &timeout)
            };
            if count < 0 {
                return Err(os_error(
                    "poll process exit watch",
                    io::Error::last_os_error(),
                ));
            }
            if count > 0 {
                if event.flags & libc::EV_ERROR != 0 {
                    return Err(os_error(
                        "process exit event",
                        io::Error::from_raw_os_error(event.data as i32),
                    ));
                }
                // EV_EOF is normal for NOTE_EXIT, not an inspection failure.
                if event.ident != self.identity.pid as libc::uintptr_t
                    || event.filter != libc::EVFILT_PROC
                    || event.fflags & libc::NOTE_EXIT == 0
                {
                    return Err("unexpected process exit event".into());
                }
            }
            if count > 0 {
                // Preserve the kernel evidence even if the following identity
                // inspection is temporarily unavailable; this call still returns
                // that inspection error, and the next poll retains the event.
                self.observed_exit = true;
            }
            let alive = is_alive(&self.identity)?;
            self.observed_exit |= !alive;
            Ok(self.observed_exit)
        }
    }

    fn process_ids(kind: u32, value: u32) -> Result<Vec<u32>, String> {
        let mut capacity = 128usize;
        for _ in 0..8 {
            let size = capacity * size_of::<libc::pid_t>();
            if size > MAX_INSPECTION_BYTES {
                return Err("process enumeration exceeds inspection bounds".into());
            }
            let mut pids = vec![0 as libc::pid_t; capacity];
            let count = unsafe {
                *libc::__error() = 0;
                libc::proc_listpids(kind, value, pids.as_mut_ptr().cast(), size as i32)
            };
            let error = io::Error::last_os_error();
            if count < 0 || (count == 0 && error.raw_os_error() != Some(0)) {
                return Err(os_error("enumerate terminal processes", error));
            }
            if count as usize > size || !(count as usize).is_multiple_of(size_of::<libc::pid_t>()) {
                return Err("process enumeration returned an invalid length".into());
            }
            if count as usize == size {
                capacity *= 2;
                continue;
            }
            pids.truncate(count as usize / size_of::<libc::pid_t>());
            if pids.iter().any(|pid| *pid < 0) {
                return Err("process enumeration returned an invalid PID".into());
            }
            return Ok(pids
                .into_iter()
                .filter(|pid| *pid > 0)
                .map(|pid| pid as u32)
                .collect());
        }
        Err("process list did not stabilize within inspection bounds".into())
    }

    fn shell_executable(pid: u32) -> Result<PathBuf, String> {
        let path = executable_path(pid)?;
        if !matches!(
            path.file_name().and_then(OsStr::to_str),
            Some(
                "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "csh" | "tcsh" | "nu" | "elvish"
            )
        ) {
            return Err("companion process is not a recognized shell".into());
        }
        Ok(path)
    }

    fn foreground_shell(info: &libc::proc_bsdinfo, foreground_pgid: i32) -> bool {
        matches!(info.pbi_status, libc::SRUN | libc::SSLEEP)
            && info.pbi_pgid == info.pbi_pid
            && info.e_tpgid == info.pbi_pgid
            && foreground_pgid as u32 == info.pbi_pgid
    }

    /// Conservative snapshot, not proof of an empty command line. The caller
    /// must also check its owned PTY child, session generation and input epoch.
    /// Any direct child means a descendant exists; any other same-TTY process
    /// also means busy, including stopped, reparented and unreaped jobs.
    pub fn shell_is_idle(shell_pid: u32, foreground_pgid: Option<i32>) -> Result<bool, String> {
        let Some(group) = foreground_pgid.filter(|group| *group > 0) else {
            return Err("companion foreground process group is unavailable".into());
        };
        let Some(before) = process_info(shell_pid)? else {
            return Ok(false);
        };
        if !same_user(&before) {
            return Err("companion shell does not belong to the current user".into());
        }
        if !foreground_shell(&before, group) {
            return Ok(false);
        }
        let tty = tty_device(&before)
            .ok_or_else(|| "companion shell has no verifiable controlling TTY".to_string())?;
        let session = session_id(shell_pid)?;
        let executable = shell_executable(shell_pid)?;
        let identity = identity_of(&before);
        // Two short passes reduce races with asynchronous shell startup jobs.
        // Reparented processes that also detached from the TTY are unknowable.
        for _ in 0..2 {
            if !process_ids(PROC_PPID_ONLY, shell_pid)?.is_empty() {
                return Ok(false);
            }
            let tty_processes = process_ids(PROC_TTY_ONLY, tty)?;
            if !tty_processes.contains(&shell_pid) {
                return Err("companion shell is missing from its TTY process list".into());
            }
            if tty_processes.iter().any(|pid| *pid != shell_pid) {
                return Ok(false);
            }
            let after = live_info(&identity)?;
            if !same_relationship(&before, &after)
                || !foreground_shell(&after, group)
                || session_id(shell_pid)? != session
            {
                return Err("companion shell changed during activity inspection".into());
            }
        }
        if shell_executable(shell_pid)? != executable {
            return Err("companion shell executable changed during inspection".into());
        }
        live_info(&identity)?;
        Ok(true)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs::File;
        use std::io::{Read, Write};
        use std::os::unix::fs::symlink;
        use std::os::unix::process::CommandExt;
        use std::process::{Child, Command, Stdio};
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::thread;
        use std::time::{Duration, Instant};

        struct TestDirectory(PathBuf);

        impl TestDirectory {
            fn new() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                loop {
                    let path = std::env::temp_dir().join(format!(
                        "monoterm-process-monitor-{}-{}",
                        std::process::id(),
                        NEXT.fetch_add(1, Ordering::Relaxed)
                    ));
                    match std::fs::create_dir(&path) {
                        Ok(()) => return Self(path.canonicalize().unwrap()),
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                        Err(error) => panic!("create test directory: {error}"),
                    }
                }
            }
        }

        impl Drop for TestDirectory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        struct TestProcess(Child);

        impl TestProcess {
            fn sleep(directory: &Path) -> Self {
                Self(
                    Command::new("/bin/sleep")
                        .arg("600")
                        .current_dir(directory)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .unwrap(),
                )
            }

            fn identity(&self) -> ProcessIdentity {
                identity_of(&process_info(self.0.id()).unwrap().unwrap())
            }

            fn finish(&mut self) {
                self.0.kill().unwrap();
                self.0.wait().unwrap();
            }
        }

        impl Drop for TestProcess {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        fn eventually(mut predicate: impl FnMut() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if predicate() {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "test process condition timed out"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }

        fn signal(identity: &ProcessIdentity, signal: i32) {
            assert!(is_alive(identity).unwrap());
            assert_eq!(unsafe { libc::kill(identity.pid as i32, signal) }, 0);
        }

        struct TestShell {
            process: TestProcess,
            master: File,
            jobs: Vec<ProcessIdentity>,
        }

        impl TestShell {
            fn new(directory: &Path) -> Self {
                let (mut master, mut slave) = (-1, -1);
                assert_eq!(
                    unsafe {
                        libc::openpty(
                            &mut master,
                            &mut slave,
                            ptr::null_mut(),
                            ptr::null_mut(),
                            ptr::null_mut(),
                        )
                    },
                    0
                );
                let master = unsafe { File::from_raw_fd(master) };
                let slave = unsafe { File::from_raw_fd(slave) };
                for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
                    assert_eq!(
                        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                        0
                    );
                }
                let mut command = Command::new("/bin/sh");
                command
                    .arg("-i")
                    .current_dir(directory)
                    .env("ENV", "/dev/null")
                    .env("PS1", "")
                    .env("MONOTERM_TEST_EXECUTABLE", directory.join("claude"))
                    .stdin(Stdio::from(slave.try_clone().unwrap()))
                    .stdout(Stdio::from(slave.try_clone().unwrap()))
                    .stderr(Stdio::from(slave));
                unsafe {
                    command.pre_exec(|| {
                        if libc::setsid() < 0
                            || libc::ioctl(0, libc::TIOCSCTTY as libc::c_ulong, 0) < 0
                            || libc::tcsetpgrp(0, libc::getpid()) < 0
                        {
                            return Err(io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                let process = TestProcess(command.spawn().unwrap());
                let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
                assert!(flags >= 0);
                assert_eq!(
                    unsafe {
                        libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK)
                    },
                    0
                );
                let mut shell = Self {
                    process,
                    master,
                    jobs: Vec::new(),
                };
                shell.send("printf '\\137ready\\137\\n'\n");
                shell.read_until("_ready_\r\n");
                shell
            }

            fn pid(&self) -> u32 {
                self.process.0.id()
            }

            fn foreground(&self) -> Option<i32> {
                let group = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
                (group > 0).then_some(group)
            }

            fn idle(&self) -> Result<bool, String> {
                shell_is_idle(self.pid(), self.foreground())
            }

            fn send(&mut self, command: &str) {
                self.master.write_all(command.as_bytes()).unwrap();
            }

            fn read_until(&mut self, marker: &str) -> String {
                let mut output = Vec::new();
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let mut buffer = [0u8; 4096];
                    match self.master.read(&mut buffer) {
                        Ok(size) => output.extend_from_slice(&buffer[..size]),
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(error) => panic!("read fixture PTY: {error}"),
                    }
                    if String::from_utf8_lossy(&output).contains(marker) {
                        return String::from_utf8_lossy(&output).into_owned();
                    }
                    assert!(
                        Instant::now() < deadline,
                        "fixture output: {:?}",
                        String::from_utf8_lossy(&output)
                    );
                    thread::sleep(Duration::from_millis(5));
                }
            }

            fn background(&mut self) -> ProcessIdentity {
                // The executable is a test-owned pause loop, never a real agent.
                self.send("\"$MONOTERM_TEST_EXECUTABLE\" 600 & printf '\\137job:%s\\137\\n' $!\n");
                let output = self.read_until("_\r\n");
                let start = output.find("_job:").expect("fixture child PID") + 5;
                let end = output[start..].find('_').unwrap() + start;
                let pid = output[start..end].parse().unwrap();
                let identity = identity_of(
                    &process_info(pid)
                        .unwrap()
                        .unwrap_or_else(|| panic!("fixture disappeared: {output:?}")),
                );
                self.jobs.push(identity.clone());
                eventually(|| {
                    executable_path(pid)
                        .is_ok_and(|path| path.file_name() != Some(OsStr::new("sh")))
                });
                identity
            }
        }

        impl Drop for TestShell {
            fn drop(&mut self) {
                // Signal only identities launched by this test. Never target a
                // user's process group, shell, Herdr instance or real agent.
                for identity in &self.jobs {
                    if is_alive(identity) == Ok(true) {
                        unsafe {
                            libc::kill(identity.pid as i32, libc::SIGKILL);
                        }
                    }
                }
            }
        }

        fn native_fixture(directory: &Path, versioned: bool) {
            let executable = if versioned {
                let versions = directory.join("claude-install/claude/versions");
                std::fs::create_dir_all(&versions).unwrap();
                versions.join("2.1.310")
            } else {
                directory.join("claude")
            };
            // Moving Apple's platform binaries invalidates their execution
            // policy on some macOS versions. Build a tiny test-owned executable
            // so proc_pidpath has a real native basename/version path to inspect.
            let mut compiler = Command::new("/usr/bin/cc")
                .args(["-x", "c", "-o"])
                .arg(&executable)
                .arg("-")
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            compiler
                .stdin
                .take()
                .unwrap()
                .write_all(b"#include <unistd.h>\nint main(void) { for (;;) pause(); }\n")
                .unwrap();
            assert!(compiler.wait().unwrap().success());
            if versioned {
                symlink(executable, directory.join("claude")).unwrap();
            }
        }

        #[test]
        fn identity_guards_reused_pid_and_native_cwd() {
            let directory = TestDirectory::new();
            let mut process = TestProcess::sleep(&directory.0);
            let identity = process.identity();
            assert_eq!(is_alive(&identity), Ok(true));
            assert_eq!(process_directory(&identity).unwrap(), directory.0);
            let reused = ProcessIdentity {
                start_time_micros: (identity.start_time_micros + 1) % 1_000_000,
                ..identity.clone()
            };
            assert_eq!(is_alive(&reused), Ok(false));
            assert!(process_directory(&reused).is_err());
            assert!(ExitWatch::new(reused).is_err());
            process.finish();
            assert_eq!(is_alive(&identity), Ok(false));
            assert!(process_directory(&identity).is_err());
            assert!(ExitWatch::new(identity).is_err());
        }

        #[test]
        fn suspended_process_is_alive_and_exit_watch_latches() {
            let directory = TestDirectory::new();
            let mut process = TestProcess::sleep(&directory.0);
            let identity = process.identity();
            let mut watch = ExitWatch::new(identity.clone()).unwrap();
            assert!(!watch.exited().unwrap());
            signal(&identity, libc::SIGSTOP);
            eventually(|| process_info(identity.pid).unwrap().unwrap().pbi_status == libc::SSTOP);
            assert_eq!(is_alive(&identity), Ok(true));
            assert!(!watch.exited().unwrap());
            signal(&identity, libc::SIGCONT);
            process.finish();
            eventually(|| watch.exited().unwrap());
            assert!(watch.exited().unwrap());
            assert_eq!(is_alive(&identity), Ok(false));
        }

        #[test]
        fn native_agent_requires_ancestry_executable_and_tty() {
            let directory = TestDirectory::new();
            native_fixture(&directory.0, false);
            let mut shell = TestShell::new(&directory.0);
            eventually(|| shell.idle() == Ok(true));
            let agent = shell.background();
            assert_eq!(
                identify_agent(shell.pid(), agent.pid, "claude").unwrap(),
                agent
            );
            assert!(identify_agent(shell.pid(), agent.pid, "codex").is_err());
            let other_shell = TestShell::new(&directory.0);
            assert!(identify_agent(other_shell.pid(), agent.pid, "claude").is_err());
            let unrelated = TestProcess::sleep(&directory.0);
            assert!(identify_agent(shell.pid(), unrelated.0.id(), "claude").is_err());
            assert_eq!(shell.idle(), Ok(false));
        }

        #[test]
        fn claude_version_path_and_stopped_background_job_are_supported() {
            let directory = TestDirectory::new();
            native_fixture(&directory.0, true);
            let mut shell = TestShell::new(&directory.0);
            eventually(|| shell.idle() == Ok(true));
            assert!(shell_is_idle(shell.pid(), None).is_err());
            assert_eq!(shell_is_idle(shell.pid(), Some(1)), Ok(false));
            let agent = shell.background();
            assert_eq!(
                identify_agent(shell.pid(), agent.pid, "claude").unwrap(),
                agent
            );
            assert_eq!(shell.idle(), Ok(false));
            signal(&agent, libc::SIGSTOP);
            eventually(|| process_info(agent.pid).unwrap().unwrap().pbi_status == libc::SSTOP);
            assert_eq!(is_alive(&agent), Ok(true));
            assert_eq!(
                identify_agent(shell.pid(), agent.pid, "claude").unwrap(),
                agent
            );
            assert_eq!(shell.idle(), Ok(false));
            signal(&agent, libc::SIGKILL);
            shell.send("wait; printf '\\137reaped\\137\\n'\n");
            shell.read_until("_reaped_\r\n");
            eventually(|| shell.idle() == Ok(true));
            let shell_identity = shell.process.identity();
            signal(&shell_identity, libc::SIGSTOP);
            eventually(|| process_info(shell.pid()).unwrap().unwrap().pbi_status == libc::SSTOP);
            assert_eq!(shell.idle(), Ok(false));
            signal(&shell_identity, libc::SIGCONT);
        }

        #[test]
        fn argv0_without_a_matching_executable_is_rejected() {
            let child = TestProcess(
                Command::new("/bin/sleep")
                    .arg0("claude")
                    .arg("600")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
            assert_eq!(argv0(child.0.id()).unwrap(), PathBuf::from("claude"));
            assert!(validate_executable(child.0.id(), "claude").is_err());
        }

        #[test]
        fn exit_watch_registers_native_exit_events_and_cloexec() {
            let directory = TestDirectory::new();
            let process = TestProcess::sleep(&directory.0);
            let watch = ExitWatch::new(process.identity()).unwrap();
            let flags = unsafe { libc::fcntl(watch.fd.as_raw_fd(), libc::F_GETFD) };
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
            // Observe normal kernel NOTE_EXIT behavior directly as well as the
            // identity polling fallback exercised by the other watcher test.
            signal(&process.identity(), libc::SIGKILL);
            eventually(|| {
                let mut event = unsafe { mem::zeroed::<libc::kevent>() };
                let zero = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                let count = unsafe {
                    libc::kevent(watch.fd.as_raw_fd(), ptr::null(), 0, &mut event, 1, &zero)
                };
                assert!(count >= 0);
                if count == 1 {
                    assert_eq!({ event.ident }, process.0.id() as usize);
                    assert_ne!(event.fflags & libc::NOTE_EXIT, 0);
                    true
                } else {
                    false
                }
            });
        }

        #[test]
        fn invalid_identities_are_unknown_and_version_paths_are_narrow() {
            let invalid = ProcessIdentity {
                pid: 0,
                start_time_secs: 1,
                start_time_micros: 0,
            };
            assert!(is_alive(&invalid).is_err());
            assert!(identify_agent(0, 1, "claude").is_err());
            assert!(!claude_version_path(Path::new("/tmp/versions/2.1.310")));
            assert!(!claude_version_path(Path::new(
                "/tmp/claude/versions/sleep"
            )));
            assert!(claude_version_path(Path::new(
                "/tmp/claude/versions/2.1.310"
            )));
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{PathBuf, ProcessIdentity};

    const UNSUPPORTED: &str = "process inspection is supported only on macOS";

    pub fn identify_agent(
        _shell_pid: u32,
        _candidate_pid: u32,
        _agent_kind: &str,
    ) -> Result<ProcessIdentity, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn process_directory(_identity: &ProcessIdentity) -> Result<PathBuf, String> {
        Err(UNSUPPORTED.into())
    }

    pub fn is_alive(_identity: &ProcessIdentity) -> Result<bool, String> {
        Err(UNSUPPORTED.into())
    }

    pub struct ExitWatch;

    impl ExitWatch {
        pub fn new(_identity: ProcessIdentity) -> Result<Self, String> {
            Err(UNSUPPORTED.into())
        }

        pub fn exited(&mut self) -> Result<bool, String> {
            Err(UNSUPPORTED.into())
        }
    }

    pub fn shell_is_idle(_shell_pid: u32, _foreground_pgid: Option<i32>) -> Result<bool, String> {
        Err(UNSUPPORTED.into())
    }
}
