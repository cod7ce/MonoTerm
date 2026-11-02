// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Fabien Penso
// Braille frames and dot geometry adapted from Herdr UI's sidebar/animated.rs.
use crate::appearance::Palette;
use gpui::{Div, div, prelude::*, px, rgb};
use herdr_protocol::AgentStatus;

const WORKING: [u8; 10] = [0x0b, 0x19, 0x39, 0x38, 0x3c, 0x34, 0x26, 0x27, 0x07, 0x0f];
const BLOCKED: [u8; 14] = [
    0xff, 0xf7, 0xf6, 0xe6, 0xe4, 0xc4, 0xc0, 0x40, 0xc0, 0xc4, 0xe4, 0xe6, 0xf6, 0xf7,
];
const BITS: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

fn frame(status: AgentStatus, elapsed_ms: u128) -> (u8, usize) {
    match status {
        AgentStatus::Working => (
            WORKING[(elapsed_ms / 80 % WORKING.len() as u128) as usize],
            3,
        ),
        AgentStatus::Blocked => (
            BLOCKED[(elapsed_ms / 70 % BLOCKED.len() as u128) as usize],
            4,
        ),
        AgentStatus::Done => (0x3f, 3),
        AgentStatus::Idle => (0x12, 3),
        AgentStatus::Unknown => (0x02, 3),
    }
}

pub(super) fn moving(status: AgentStatus) -> bool {
    matches!(status, AgentStatus::Working | AgentStatus::Blocked)
}

pub(super) fn color(status: AgentStatus, palette: Palette) -> u32 {
    let color = match status {
        AgentStatus::Working => 0xf9e2af,
        AgentStatus::Blocked => 0xf38ba8,
        AgentStatus::Done => 0x94e2d5,
        AgentStatus::Idle => 0xa6e3a1,
        AgentStatus::Unknown => 0x6c7086,
    };
    if palette.dark {
        color
    } else {
        let channel = |shift: u32| ((color >> shift) & 255u32) * 45 / 100;
        (channel(16) << 16) | (channel(8) << 8) | channel(0)
    }
}

pub(super) fn mark(status: AgentStatus, elapsed_ms: u128, color: u32) -> Div {
    let (bits, rows) = frame(status, elapsed_ms);
    div()
        .size(px(14.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(1.0))
                .children(BITS.into_iter().take(rows).map(|row| {
                    div().flex().gap(px(1.0)).children(row.map(|bit| {
                        div()
                            .size(px(2.5))
                            .flex_none()
                            .rounded_full()
                            .when(bits & bit != 0, |dot| dot.bg(rgb(color)))
                    }))
                })),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braille_states_have_distinct_frames_and_wrap() {
        assert_ne!(
            frame(AgentStatus::Working, 0),
            frame(AgentStatus::Working, 80)
        );
        assert_eq!(
            frame(AgentStatus::Working, 0),
            frame(AgentStatus::Working, 800)
        );
        assert_eq!(frame(AgentStatus::Blocked, 0), (0xff, 4));
        assert_ne!(
            frame(AgentStatus::Blocked, 0),
            frame(AgentStatus::Blocked, 70)
        );
        assert_eq!(frame(AgentStatus::Done, 0), (0x3f, 3));
        assert_eq!(frame(AgentStatus::Idle, 800), (0x12, 3));
        assert_eq!(frame(AgentStatus::Unknown, 800), (0x02, 3));
    }
}
