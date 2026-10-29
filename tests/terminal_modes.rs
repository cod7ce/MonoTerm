use gpui_ghostty_terminal::{TerminalConfig, TerminalSession};

fn mouse_state(chunks: impl IntoIterator<Item = impl AsRef<[u8]>>) -> (bool, bool, bool, bool) {
    let mut terminal = TerminalSession::new(TerminalConfig::default()).unwrap();
    for chunk in chunks {
        terminal.feed(chunk.as_ref()).unwrap();
    }
    (
        terminal.mouse_reporting_enabled(),
        terminal.mouse_sgr_enabled(),
        terminal.mouse_any_event_enabled(),
        terminal.bracketed_paste_enabled(),
    )
}

#[test]
fn first_screen_preserves_mouse_modes_regardless_of_output_chunking() {
    let mut output = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h\x1b[?2004h".to_vec();
    output.extend_from_slice(&vec![b' '; 8192]);
    for chunk_size in [1, 7, 2048, 4096, output.len()] {
        assert_eq!(
            mouse_state(output.chunks(chunk_size)),
            (true, true, true, true),
            "output chunk size {chunk_size}"
        );
    }
}

#[test]
fn final_screen_disables_mouse_modes_before_large_output() {
    let enabled = b"\x1b[?1000;1002;1003;1006;2004h";
    let mut disabled = b"\x1b[?1000;1002;1003;1006;2004l".to_vec();
    disabled.extend_from_slice(&vec![b' '; 8192]);
    assert_eq!(
        mouse_state([enabled.as_slice(), disabled.as_slice()]),
        (false, false, false, false)
    );
}
