use super::*;
use herdr_protocol::{ClientShellPopupSurface, FrameData, PaneSurfacePane};

fn fixture() -> (ClientShellSnapshot, PaneSurfaceFrame) {
    let mut snapshot: ClientShellSnapshot = serde_json::from_str(include_str!(
        "../vendor/herdr/herdr-protocol/tests/fixtures/endpoint-snapshot-v1.json"
    ))
    .unwrap();
    let mut pane = snapshot.panes[0].clone();
    pane.pane_id = "w1:p2".into();
    pane.focused = false;
    snapshot.panes.push(pane);
    let rect = SurfaceRect {
        x: 0,
        y: 0,
        width: 80,
        height: 24,
    };
    let frame = PaneSurfaceFrame {
        boot_id: snapshot.boot_id.clone(),
        projection_revision: snapshot.revision,
        surface_revision: 1,
        frame: FrameData {
            width: 80,
            height: 24,
            cells: vec![],
            cursor: None,
            hyperlinks: vec![],
            graphics: vec![],
        },
        panes: snapshot
            .panes
            .iter()
            .map(|pane| PaneSurfacePane {
                pane_id: pane.pane_id.clone(),
                content_revision: 1,
                rect,
                inner_rect: rect,
                scrollbar_rect: None,
                scroll: None,
                focused: pane.focused,
                mouse_reporting: false,
                sgr_pixel_mouse: false,
                alternate_screen_active: false,
                pixel_width: 0,
                pixel_height: 0,
            })
            .collect(),
        splits: vec![PaneSurfaceSplit {
            direction: PaneSurfaceSplitDirection::Horizontal,
            pos: 40,
            area: rect,
            hit_rect: SurfaceRect {
                x: 40,
                y: 0,
                width: 1,
                height: 24,
            },
            path: vec![],
        }],
        popup: None,
        graphics: Default::default(),
    };
    (snapshot, frame)
}

#[test]
fn split_ratios_use_the_split_area_axis_and_clamp_outside_window() {
    let (snapshot, mut surface) = fixture();
    let target = SplitTarget::capture(&snapshot, &surface, &surface.splits[0]).unwrap();
    assert_eq!(target.edge(8.0, 20.0), 320.0);
    assert_eq!(target.ratio(480.0, 8.0, 20.0), Some(0.75));
    assert_eq!(target.ratio(-100.0, 8.0, 20.0), Some(0.1));
    assert_eq!(target.ratio(2000.0, 8.0, 20.0), Some(0.9));
    assert_eq!(target.ratio(f32::NAN, 8.0, 20.0), None);
    assert_eq!(target.ratio(480.0, 0.0, 20.0), None);
    surface.splits[0].direction = PaneSurfaceSplitDirection::Vertical;
    surface.splits[0].area = SurfaceRect {
        x: 40,
        y: 4,
        width: 40,
        height: 20,
    };
    surface.splits[0].path = vec![true];
    surface.splits[0].pos = 14;
    let target = SplitTarget::capture(&snapshot, &surface, &surface.splits[0]).unwrap();
    assert_eq!(target.edge(8.0, 20.0), 280.0);
    assert_eq!(target.ratio(380.0, 8.0, 20.0), Some(0.75));
}

#[test]
fn split_identity_survives_output_and_own_ratio_but_rejects_reused_paths() {
    let (snapshot, surface) = fixture();
    let target = SplitTarget::capture(&snapshot, &surface, &surface.splits[0]).unwrap();
    let mut next = surface.clone();
    next.surface_revision += 1;
    next.panes[0].content_revision += 1;
    next.splits[0].pos = 50;
    assert!(target.validate(&snapshot, &next).is_ok());
    for change in 0..9 {
        let mut snapshot = snapshot.clone();
        let mut changed = surface.clone();
        match change {
            0 => snapshot.boot_id = "restarted".into(),
            1 => snapshot.focused_tab_id = Some("w1:t2".into()),
            2 => changed.panes.reverse(),
            3 => {
                changed.panes.pop();
            }
            4 => changed.splits[0].direction = PaneSurfaceSplitDirection::Vertical,
            5 => changed.splits[0].area.width -= 1,
            6 => snapshot.tabs[0].zoomed = true,
            7 => {
                changed.popup = Some(Box::new(ClientShellPopupSurface {
                    terminal_id: "popup".into(),
                    title: "Popup".into(),
                    frame: changed.frame.clone(),
                    mouse_reporting: false,
                    sgr_pixel_mouse: false,
                    pixel_width: 0,
                    pixel_height: 0,
                    width: None,
                    height: None,
                }))
            }
            _ => changed.projection_revision += 1,
        }
        assert!(
            target.validate(&snapshot, &changed).is_err(),
            "change {change}"
        );
    }
}
