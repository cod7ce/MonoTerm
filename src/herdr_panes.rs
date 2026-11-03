//! Stable identities for daemon-owned pane names and draggable split borders.
#[cfg(test)]
#[path = "herdr_panes_tests.rs"]
mod tests;
use herdr_protocol::{
    ClientShellSnapshot, PaneSurfaceFrame, PaneSurfaceSplit, PaneSurfaceSplitDirection, SurfaceRect,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneTarget {
    pub(crate) boot_id: String,
    pub(crate) pane: herdr_protocol::ClientShellPane,
}

impl PaneTarget {
    pub fn new(snapshot: &ClientShellSnapshot, pane: &herdr_protocol::ClientShellPane) -> Self {
        Self {
            boot_id: snapshot.boot_id.clone(),
            pane: pane.clone(),
        }
    }

    pub fn label(&self) -> Option<&str> {
        self.pane.label.as_deref()
    }

    pub fn validate(&self, snapshot: &ClientShellSnapshot) -> Result<(), String> {
        if snapshot.boot_id != self.boot_id
            || !snapshot
                .workspaces
                .iter()
                .any(|workspace| workspace.workspace_id == self.pane.workspace_id)
            || !snapshot.tabs.iter().any(|tab| {
                tab.tab_id == self.pane.tab_id && tab.workspace_id == self.pane.workspace_id
            })
            || !snapshot.panes.iter().any(|pane| {
                pane.pane_id == self.pane.pane_id
                    && pane.tab_id == self.pane.tab_id
                    && pane.workspace_id == self.pane.workspace_id
            })
        {
            return Err("Herdr pane destination changed; reopen its menu".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Topology {
    // Keep render/tree order so exchanging panes also ends a held drag.
    panes: Vec<String>,
    splits: Vec<(Vec<bool>, bool)>,
}

impl Topology {
    fn new(surface: &PaneSurfaceFrame) -> Self {
        let mut splits: Vec<_> = surface
            .splits
            .iter()
            .map(|split| {
                (
                    split.path.clone(),
                    split.direction == PaneSurfaceSplitDirection::Vertical,
                )
            })
            .collect();
        splits.sort();
        Self {
            panes: surface
                .panes
                .iter()
                .map(|pane| pane.pane_id.clone())
                .collect(),
            splits,
        }
    }
}

/// A split path only has meaning within its captured tab and pane tree.
#[derive(Clone, Debug)]
pub struct SplitTarget {
    pub(crate) boot_id: String,
    pub(crate) tab_id: String,
    workspace_id: String,
    pub(crate) split: PaneSurfaceSplit,
    topology: Topology,
}

impl SplitTarget {
    pub fn capture(
        snapshot: &ClientShellSnapshot,
        surface: &PaneSurfaceFrame,
        split: &PaneSurfaceSplit,
    ) -> Option<Self> {
        let tab_id = snapshot.focused_tab_id.as_ref()?;
        let target = Self {
            boot_id: snapshot.boot_id.clone(),
            tab_id: tab_id.clone(),
            workspace_id: snapshot.focused_workspace_id.clone()?,
            split: split.clone(),
            topology: Topology::new(surface),
        };
        target.validate(snapshot, surface).ok()?;
        Some(target)
    }

    pub fn validate(
        &self,
        snapshot: &ClientShellSnapshot,
        surface: &PaneSurfaceFrame,
    ) -> Result<(), String> {
        if snapshot.boot_id != self.boot_id
            || snapshot.focused_tab_id.as_ref() != Some(&self.tab_id)
            || snapshot.focused_workspace_id.as_ref() != Some(&self.workspace_id)
            || !snapshot
                .workspaces
                .iter()
                .any(|workspace| workspace.workspace_id == self.workspace_id)
            || !snapshot.tabs.iter().any(|tab| {
                tab.tab_id == self.tab_id && tab.workspace_id == self.workspace_id && !tab.zoomed
            })
            || surface.boot_id != self.boot_id
            || surface.projection_revision != snapshot.revision
            || surface.popup.is_some()
            || Topology::new(surface) != self.topology
            || !surface.panes.iter().all(|pane| {
                snapshot.panes.iter().any(|p| {
                    p.pane_id == pane.pane_id
                        && p.tab_id == self.tab_id
                        && p.workspace_id == self.workspace_id
                })
            })
            || !surface.splits.iter().any(|split| {
                split.path == self.split.path
                    && split.direction == self.split.direction
                    && split.area == self.split.area
            })
        {
            return Err("Herdr pane layout changed; drag again".into());
        }
        Ok(())
    }

    pub fn pointer(&self, x: f32, y: f32) -> f32 {
        match self.split.direction {
            PaneSurfaceSplitDirection::Horizontal => x,
            PaneSurfaceSplitDirection::Vertical => y,
        }
    }

    pub fn edge(&self, cell_width: f32, cell_height: f32) -> f32 {
        f32::from(self.split.pos) * self.pointer(cell_width, cell_height)
    }

    pub fn ratio(&self, position: f32, cell_width: f32, cell_height: f32) -> Option<f32> {
        let SurfaceRect {
            x,
            y,
            width,
            height,
        } = self.split.area;
        let origin = self.pointer(f32::from(x), f32::from(y));
        let length = self.pointer(f32::from(width), f32::from(height));
        let cell = self.pointer(cell_width, cell_height);
        if !position.is_finite() || !cell.is_finite() || cell <= 0.0 || length <= 0.0 {
            return None;
        }
        Some(((position / cell - origin) / length).clamp(0.1, 0.9))
    }
}
