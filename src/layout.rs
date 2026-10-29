/// The direction in which a split arranges its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Place the first child to the left of the second child.
    Horizontal,
    /// Place the first child above the second child.
    Vertical,
}

/// A nonempty pane tree. Pane IDs and split IDs have separate namespaces.
#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Leaf(usize),
    Split {
        id: usize,
        axis: Axis,
        /// The fraction of the available space occupied by the first child.
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    pub fn new(pane_id: usize) -> Self {
        Self::Leaf(pane_id)
    }

    /// Return pane IDs in first-child-before-second-child traversal order.
    pub fn pane_ids(&self) -> Vec<usize> {
        fn collect(layout: &Layout, ids: &mut Vec<usize>) {
            match layout {
                Layout::Leaf(id) => ids.push(*id),
                Layout::Split { first, second, .. } => {
                    collect(first, ids);
                    collect(second, ids);
                }
            }
        }

        let mut ids = Vec::new();
        collect(self, &mut ids);
        ids
    }

    pub fn pane_count(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    pub fn contains(&self, pane_id: usize) -> bool {
        match self {
            Self::Leaf(id) => *id == pane_id,
            Self::Split { first, second, .. } => {
                first.contains(pane_id) || second.contains(pane_id)
            }
        }
    }

    pub fn first_pane(&self) -> usize {
        match self {
            Self::Leaf(id) => *id,
            Self::Split { first, .. } => first.first_pane(),
        }
    }

    /// Replace a pane with an equal split, keeping it first and adding the new pane second.
    /// Missing targets or reused pane/split IDs leave the layout unchanged.
    pub fn split(&mut self, target: usize, new_id: usize, split_id: usize, axis: Axis) -> bool {
        if self.contains(new_id) || self.contains_split(split_id) {
            return false;
        }
        self.split_leaf(target, new_id, split_id, axis)
    }

    fn split_leaf(&mut self, target: usize, new_id: usize, split_id: usize, axis: Axis) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split {
                    id: split_id,
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Leaf(target)),
                    second: Box::new(Self::Leaf(new_id)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split_leaf(target, new_id, split_id, axis)
                    || second.split_leaf(target, new_id, split_id, axis)
            }
        }
    }

    fn contains_split(&self, split_id: usize) -> bool {
        match self {
            Self::Leaf(_) => false,
            Self::Split {
                id, first, second, ..
            } => {
                *id == split_id || first.contains_split(split_id) || second.contains_split(split_id)
            }
        }
    }

    /// Remove a pane, promoting its sibling when a split loses a child.
    /// Return `None` only when the final pane is removed; missing targets preserve the tree.
    pub fn remove(self, target: usize) -> Option<Self> {
        match self {
            Self::Leaf(id) => (id != target).then_some(Self::Leaf(id)),
            Self::Split {
                id,
                axis,
                ratio,
                first,
                second,
            } => match ((*first).remove(target), (*second).remove(target)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    id,
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(remaining), None) | (None, Some(remaining)) => Some(remaining),
                (None, None) => None,
            },
        }
    }

    /// Set a split's first-child fraction, clamping finite values to 0.15..=0.85.
    /// Return false without mutation for nonfinite values or missing split IDs.
    pub fn set_ratio(&mut self, split_id: usize, ratio: f32) -> bool {
        if !ratio.is_finite() {
            return false;
        }
        match self {
            Self::Leaf(_) => false,
            Self::Split {
                id,
                ratio: current,
                first,
                second,
                ..
            } => {
                if *id == split_id {
                    *current = ratio.clamp(0.15, 0.85);
                    true
                } else {
                    first.set_ratio(split_id, ratio) || second.set_ratio(split_id, ratio)
                }
            }
        }
    }

    fn sibling_pane(&self, target: usize) -> Option<usize> {
        match self {
            Self::Leaf(_) => None,
            Self::Split { first, second, .. } => {
                if matches!(first.as_ref(), Self::Leaf(id) if *id == target) {
                    Some(second.first_pane())
                } else if matches!(second.as_ref(), Self::Leaf(id) if *id == target) {
                    Some(first.first_pane())
                } else {
                    first
                        .sibling_pane(target)
                        .or_else(|| second.sibling_pane(target))
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabIcon {
    Terminal,
    Code,
    Server,
    Database,
    Globe,
    Star,
    Bolt,
    Branch,
}

impl TabIcon {
    pub const ALL: [Self; 8] = [
        Self::Terminal,
        Self::Code,
        Self::Server,
        Self::Database,
        Self::Globe,
        Self::Star,
        Self::Bolt,
        Self::Branch,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Code => "code",
            Self::Server => "server",
            Self::Database => "database",
            Self::Globe => "globe",
            Self::Star => "star",
            Self::Bolt => "bolt",
            Self::Branch => "branch",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Terminal => "Terminal",
            Self::Code => "Code",
            Self::Server => "Server",
            Self::Database => "Database",
            Self::Globe => "Globe",
            Self::Star => "Star",
            Self::Bolt => "Lightning",
            Self::Branch => "Branch",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabAppearance {
    pub title: Option<String>,
    pub icon: Option<TabIcon>,
}

impl TabAppearance {
    pub fn new(title: &str, icon: Option<TabIcon>) -> Self {
        let title: String = title.chars().filter(|c| !c.is_control()).collect();
        let title = title.trim();
        Self {
            // A pasted title must not become an unbounded label or window title.
            title: (!title.is_empty()).then(|| title.chars().take(120).collect()),
            icon,
        }
    }
}

/// A nonempty tab whose active pane belongs to its layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub id: usize,
    pub layout: Layout,
    pub active_pane: usize,
    pub appearance: TabAppearance,
}

impl Tab {
    pub fn new(id: usize, pane_id: usize) -> Self {
        Self {
            id,
            layout: Layout::new(pane_id),
            active_pane: pane_id,
            appearance: TabAppearance::default(),
        }
    }

    pub fn pane_ids(&self) -> Vec<usize> {
        self.layout.pane_ids()
    }

    pub fn pane_count(&self) -> usize {
        self.layout.pane_count()
    }

    /// Split the active pane and focus the new pane on success.
    pub fn split(&mut self, new_id: usize, split_id: usize, axis: Axis) -> bool {
        if self.layout.split(self.active_pane, new_id, split_id, axis) {
            self.active_pane = new_id;
            true
        } else {
            false
        }
    }

    /// Close a pane, focusing the first pane of its sibling subtree if it was active.
    /// Closing the final pane returns `None`; closing a missing pane changes nothing.
    pub fn close(self, target: usize) -> Option<Self> {
        let active_pane = if self.active_pane == target {
            self.layout.sibling_pane(target).unwrap_or(self.active_pane)
        } else {
            self.active_pane
        };
        Some(Self {
            id: self.id,
            layout: self.layout.remove(target)?,
            active_pane,
            appearance: self.appearance,
        })
    }

    pub fn close_active(self) -> Option<Self> {
        let target = self.active_pane;
        self.close(target)
    }

    /// Move focus in traversal order, wrapping at either end, and return the focused ID.
    pub fn cycle_focus(&mut self, forward: bool) -> usize {
        let ids = self.pane_ids();
        let index = match ids.iter().position(|id| *id == self.active_pane) {
            Some(index) if forward => (index + 1) % ids.len(),
            Some(index) => (index + ids.len() - 1) % ids.len(),
            None if forward => 0,
            None => ids.len() - 1,
        };
        self.active_pane = ids[index];
        self.active_pane
    }
}

#[cfg(test)]
mod tests {
    use super::{Axis, Layout, Tab};
    use Axis::{Horizontal, Vertical};

    fn branch(id: usize, axis: Axis, ratio: f32, first: Layout, second: Layout) -> Layout {
        Layout::Split {
            id,
            axis,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    #[test]
    fn nested_removal_preserves_siblings_axes_and_ratios() {
        let mut layout = Layout::new(1);
        assert!(layout.split(1, 2, 100, Horizontal));
        assert!(layout.split(1, 3, 101, Vertical));
        assert!(layout.split(2, 4, 102, Vertical));
        assert!(layout.split(3, 5, 103, Horizontal));
        for (id, ratio) in [(100, 0.35), (101, 0.6), (102, 0.75), (103, 0.2)] {
            assert!(layout.set_ratio(id, ratio));
        }
        assert_eq!(layout.pane_ids(), [1, 3, 5, 2, 4]);
        assert_eq!(layout.pane_count(), 5);
        assert_eq!(layout.first_pane(), 1);

        let left = branch(103, Horizontal, 0.2, Layout::Leaf(3), Layout::Leaf(5));
        let right = branch(102, Vertical, 0.75, Layout::Leaf(2), Layout::Leaf(4));
        let layout = layout.remove(1).unwrap();
        assert_eq!(layout, branch(100, Horizontal, 0.35, left.clone(), right));
        assert!(!layout.contains(1));
        assert_eq!(layout.first_pane(), 3);

        let layout = layout.remove(2).unwrap();
        assert_eq!(layout, branch(100, Horizontal, 0.35, left, Layout::Leaf(4)));
        let layout = layout.remove(5).unwrap();
        assert_eq!(
            layout,
            branch(100, Horizontal, 0.35, Layout::Leaf(3), Layout::Leaf(4))
        );
        let layout = layout.remove(3).unwrap();
        assert_eq!(layout, Layout::Leaf(4));
        assert_eq!(layout.pane_count(), 1);
        assert_eq!(layout.remove(4), None);
    }

    #[test]
    fn removing_a_root_child_promotes_the_entire_sibling_tree() {
        let sibling = branch(20, Vertical, 0.7, Layout::Leaf(2), Layout::Leaf(3));
        let layout = branch(10, Horizontal, 0.3, Layout::Leaf(1), sibling.clone());
        assert_eq!(layout.remove(1), Some(sibling.clone()));
        let layout = branch(10, Horizontal, 0.3, sibling.clone(), Layout::Leaf(1));
        assert_eq!(layout.remove(1), Some(sibling));
    }

    #[test]
    fn invalid_targets_and_reused_ids_do_not_mutate_the_layout() {
        let mut layout = Layout::new(1);
        // Pane and split IDs are allowed to overlap across namespaces.
        assert!(layout.split(1, 2, 2, Horizontal));
        assert!(layout.split(2, 3, 20, Vertical));
        assert!(layout.set_ratio(20, 0.65));
        let original = layout.clone();

        assert!(!layout.split(99, 4, 30, Horizontal));
        assert_eq!(layout, original);
        assert!(!layout.split(1, 3, 30, Vertical));
        assert_eq!(layout, original);
        assert!(!layout.split(1, 4, 20, Vertical));
        assert_eq!(layout, original);
        assert!(!layout.set_ratio(99, 0.25));
        assert_eq!(layout, original);
        assert_eq!(layout.remove(99), Some(original));
    }

    #[test]
    fn ratios_clamp_finite_values_and_reject_nonfinite_values() {
        let mut layout = Layout::new(1);
        assert!(layout.split(1, 2, 10, Horizontal));
        assert!(layout.split(2, 3, 20, Vertical));
        for (input, expected) in [(-10.0, 0.15), (10.0, 0.85), (0.4, 0.4)] {
            assert!(layout.set_ratio(20, input));
            assert_eq!(
                layout,
                branch(
                    10,
                    Horizontal,
                    0.5,
                    Layout::Leaf(1),
                    branch(20, Vertical, expected, Layout::Leaf(2), Layout::Leaf(3))
                )
            );
        }
        let original = layout.clone();
        for ratio in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!layout.set_ratio(20, ratio));
            assert_eq!(layout, original);
        }
    }

    #[test]
    fn splitting_focuses_the_new_pane_only_on_success() {
        let mut tab = Tab::new(7, 1);
        assert!(tab.split(2, 10, Horizontal));
        assert_eq!(tab.active_pane, 2);
        assert!(tab.split(3, 20, Vertical));
        assert_eq!(tab.active_pane, 3);
        assert_eq!(tab.pane_ids(), [1, 2, 3]);
        assert_eq!(tab.pane_count(), 3);
        let original = tab.clone();
        assert!(!tab.split(1, 30, Horizontal));
        assert_eq!(tab, original);
        assert!(!tab.split(4, 20, Horizontal));
        assert_eq!(tab, original);

        tab.active_pane = 99;
        let original = tab.clone();
        assert!(!tab.split(4, 30, Horizontal));
        assert_eq!(tab, original);
    }

    #[test]
    fn closing_active_focuses_first_pane_in_the_nearest_sibling_subtree() {
        let mut tab = Tab::new(7, 1);
        assert!(tab.split(2, 10, Horizontal));
        assert!(tab.split(3, 20, Vertical));
        tab.active_pane = 1;
        let tab = tab.close_active().unwrap();
        assert_eq!(tab.id, 7);
        assert_eq!(tab.active_pane, 2);
        assert_eq!(tab.pane_ids(), [2, 3]);
        let tab = tab.close_active().unwrap();
        assert_eq!(tab.active_pane, 3);
        assert_eq!(tab.close_active(), None);
    }

    #[test]
    fn tab_keeps_its_identity_through_pane_changes() {
        let mut tab = Tab::new(7, 1);
        assert!(tab.split(2, 10, Horizontal));
        let tab = tab.close(2).unwrap();
        assert_eq!(tab.id, 7);
        assert_eq!(tab.pane_ids(), [1]);
    }

    #[test]
    fn customization_survives_split_focus_and_pane_removal() {
        let mut tab = Tab::new(7, 1);
        let appearance = super::TabAppearance::new("开发 🦀", Some(super::TabIcon::Code));
        tab.appearance = appearance.clone();
        assert!(tab.split(2, 10, Horizontal));
        assert!(tab.split(3, 20, Vertical));
        tab.cycle_focus(true);
        let tab = tab.close_active().unwrap();
        let tab = tab.close(3).unwrap();
        assert_eq!(tab.id, 7);
        assert_eq!(tab.appearance, appearance);
        assert_eq!(tab.pane_ids(), [2]);
        assert_eq!(Tab::new(8, 8).appearance, super::TabAppearance::default());
    }

    #[test]
    fn custom_titles_preserve_unicode_and_empty_titles_restore_default() {
        use super::{TabAppearance, TabIcon};
        assert_eq!(
            TabAppearance::new(" \t\r\n", None),
            TabAppearance::default()
        );
        assert_eq!(
            TabAppearance::new("  开发 🦀 e\u{301}\n\0  ", Some(TabIcon::Star)),
            TabAppearance {
                title: Some("开发 🦀 e\u{301}".into()),
                icon: Some(TabIcon::Star)
            }
        );
        assert_eq!(
            TabAppearance::new(&"界".repeat(200), None)
                .title
                .unwrap()
                .chars()
                .count(),
            120
        );
    }

    #[test]
    fn focus_cycles_in_both_directions_after_removal() {
        let mut tab = Tab::new(7, 1);
        assert!(tab.split(2, 10, Horizontal));
        assert!(tab.split(3, 20, Vertical));
        assert!(tab.split(4, 30, Horizontal));
        let original = tab.clone();
        assert_eq!(tab.clone().close(99), Some(original));

        let mut tab = tab.close(2).unwrap();
        assert_eq!(tab.active_pane, 4);
        assert_eq!(tab.pane_ids(), [1, 3, 4]);
        assert_eq!(tab.cycle_focus(true), 1);
        assert_eq!(tab.cycle_focus(false), 4);
        assert_eq!(tab.cycle_focus(false), 3);
        assert_eq!(tab.cycle_focus(true), 4);

        let mut tab = tab.close_active().unwrap();
        assert_eq!(tab.active_pane, 3);
        assert_eq!(tab.pane_ids(), [1, 3]);
        assert_eq!(tab.cycle_focus(true), 1);
        assert_eq!(tab.cycle_focus(true), 3);
        let mut tab = tab.close_active().unwrap();
        assert_eq!(tab.active_pane, 1);
        assert_eq!(tab.cycle_focus(true), 1);
        assert_eq!(tab.cycle_focus(false), 1);
    }
}
