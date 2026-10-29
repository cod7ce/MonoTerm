//! Opt-in binary-protocol smoke test for a caller-owned isolated Herdr session.
#![cfg(feature = "live-herdr-tests")]

use herdr_client::{ClientEvent, ConnectOptions, ConnectTarget, connect_with_surface_active};
use std::{
    path::Path,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires MONOTERM_HERDR_SOCKET pointing to an owned monoterm-client-* session"]
fn isolated_client_receives_a_daemon_surface() {
    assert_eq!(std::env::var("HERDR_ENV").as_deref(), Ok("1"));
    let source = std::env::var("MONOTERM_HERDR_SOCKET").expect("explicit owned JSON socket");
    let path = Path::new(&source);
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("herdr.sock")
    );
    assert!(
        path.parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("monoterm-client-"))
    );
    let client = connect_with_surface_active(
        ConnectTarget::Socket(path.with_file_name("herdr-client.sock")),
        ConnectOptions::default(),
        true,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut snapshot = None;
    let mut surface = None;
    while Instant::now() < deadline && (snapshot.is_none() || surface.is_none()) {
        match client.events.recv_timeout(Duration::from_millis(50)) {
            Ok(ClientEvent::Snapshot(value)) => snapshot = Some(value),
            Ok(ClientEvent::Surface(value)) => surface = Some(value),
            Ok(ClientEvent::Disconnected { reason, .. }) => {
                panic!("isolated daemon disconnected: {reason}")
            }
            _ => {}
        }
    }
    client.handle.disconnect();
    let snapshot = snapshot.expect("snapshot");
    let surface = surface.expect("surface");
    assert_eq!(surface.boot_id, snapshot.boot_id);
    assert_eq!(surface.projection_revision, snapshot.revision);
    assert!(!surface.panes.is_empty());
}
