use gpui_ghostty_terminal::{TerminalConfig, TerminalSession};

#[test]
fn primary_device_attributes_reply_survives_every_chunk_boundary() {
    // Ghostty accepts primary DA requests independently of their numeric parameter.
    for query in [b"\x1b[c".as_slice(), b"\x1b[0c", b"\x1b[1c"] {
        for boundary in 0..=query.len() {
            let mut terminal = TerminalSession::new(TerminalConfig::default()).unwrap();
            let mut replies = Vec::new();
            for chunk in [&query[..boundary], &query[boundary..]] {
                terminal
                    .feed_with_pty_responses(chunk, |bytes| replies.extend_from_slice(bytes))
                    .unwrap();
            }
            assert_eq!(
                replies, b"\x1b[?1;2c",
                "query {query:?}, boundary {boundary}"
            );
        }
    }
}

#[test]
fn upstream_secondary_and_tertiary_attributes_are_answered() {
    for (query, expected) in [
        (b"\x1b[>c".as_slice(), b"\x1b[>1;140;0c".as_slice()),
        (b"\x1b[=c".as_slice(), b"\x1bP!|00000000\x1b\\".as_slice()),
    ] {
        for boundary in 0..=query.len() {
            let mut terminal = TerminalSession::new(TerminalConfig::default()).unwrap();
            let mut replies = Vec::new();
            for chunk in [&query[..boundary], &query[boundary..]] {
                terminal
                    .feed_with_pty_responses(chunk, |bytes| replies.extend_from_slice(bytes))
                    .unwrap();
            }
            assert_eq!(replies, expected, "query {query:?}, boundary {boundary}");
        }
    }
}

#[test]
fn primary_attribute_response_does_not_loop() {
    let mut terminal = TerminalSession::new(TerminalConfig::default()).unwrap();
    let mut replies = Vec::new();
    terminal
        .feed_with_pty_responses(b"\x1b[?1;2c", |bytes| replies.extend_from_slice(bytes))
        .unwrap();
    assert!(replies.is_empty());
}

#[test]
fn kitty_query_reply_survives_every_chunk_boundary_without_echoing_payload() {
    let query = b"\x1b_Ga=q,t=d,f=32,s=1,v=1,i=41;/wAA/w==\x1b\\";
    for boundary in 0..=query.len() {
        let mut terminal = TerminalSession::new(TerminalConfig::default()).unwrap();
        let mut replies = Vec::new();
        for chunk in [&query[..boundary], &query[boundary..]] {
            terminal
                .feed_with_pty_responses(chunk, |bytes| replies.extend_from_slice(bytes))
                .unwrap();
        }
        assert_eq!(replies, b"\x1b_Gi=41;OK\x1b\\", "boundary {boundary}");
        assert!(terminal.dump_viewport().unwrap().trim().is_empty());
    }
}

// Requires a locally installed Neovim; no user config or files are loaded.
#[test]
#[ignore = "launches real Neovim in a PTY; run explicitly"]
fn neovim_exit_does_not_wait_for_the_device_attributes_timeout() {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::{
        io::{Read, Write},
        sync::mpsc,
        time::{Duration, Instant},
    };

    let run = |answer_attributes: bool| {
        let pair = native_pty_system().openpty(PtySize::default()).unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        let mut command = CommandBuilder::new("nvim");
        command.args([
            "--clean",
            "-u",
            "NONE",
            "-i",
            "NONE",
            "-n",
            "-c",
            "call timer_start(250, {-> execute('qa!')})",
        ]);
        command.env("TERM", "xterm-256color");
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let (sender, output) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut buffer = [0; 16384];
            while let Ok(length) = reader.read(&mut buffer) {
                if length == 0 || sender.send(buffer[..length].to_vec()).is_err() {
                    break;
                }
            }
        });
        let start = Instant::now();
        let mut exit_start = None;
        let mut replies = 0;
        let mut terminal = TerminalSession::new(TerminalConfig::default()).unwrap();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if start.elapsed() > Duration::from_secs(5) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("Neovim did not exit within the test deadline");
            }
            if let Ok(bytes) = output.recv_timeout(Duration::from_millis(5)) {
                terminal
                    .feed_with_pty_responses(&bytes, |response| {
                        if response == b"\x1b[?1;2c" {
                            replies += 1;
                            // The final DA1 query precedes the exit barrier;
                            // leaving the alternate screen happens after it.
                            exit_start = Some(Instant::now());
                            if !answer_attributes {
                                return;
                            }
                        }
                        writer.write_all(response).unwrap();
                        writer.flush().unwrap();
                    })
                    .unwrap();
            }
        }
        let elapsed = exit_start.expect("Neovim sent the exit query").elapsed();
        drop(writer);
        drop(pair.master);
        thread.join().unwrap();
        assert!(replies > 0, "Neovim requested device attributes");
        elapsed
    };

    let unanswered = run(false);
    let answered = run(true);
    eprintln!("Neovim exit: unanswered={unanswered:?}, answered={answered:?}");
    assert!(unanswered >= Duration::from_millis(900));
    assert!(answered < Duration::from_millis(500));
}
