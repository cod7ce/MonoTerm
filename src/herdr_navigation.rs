//! Bounded history of acknowledged daemon focus for the native Back/Forward controls.
#[derive(Default)]
pub(super) struct FocusHistory {
    boot: Option<String>,
    panes: Vec<String>,
    cursor: usize,
    pending: Option<usize>,
}

impl FocusHistory {
    pub(super) fn observe(&mut self, boot: Option<&str>, pane: Option<&str>, ready: bool) {
        if self.boot.as_deref() != boot {
            self.boot = boot.map(str::to_owned);
            self.panes.clear();
            self.cursor = 0;
            self.pending = None;
        }
        if !ready {
            return;
        }
        let Some(pane) = pane else { return };
        if let Some(index) = self.pending.take()
            && self.panes.get(index).is_some_and(|id| id == pane)
        {
            self.cursor = index;
            return;
        }
        if self.panes.get(self.cursor).is_some_and(|id| id == pane) {
            return;
        }
        self.panes.truncate(self.cursor + 1);
        self.panes.push(pane.into());
        if self.panes.len() > 64 {
            self.panes.remove(0);
        }
        self.cursor = self.panes.len() - 1;
    }

    pub(super) fn destination(
        &self,
        back: bool,
        exists: impl Fn(&str) -> bool,
    ) -> Option<(usize, String)> {
        if self.pending.is_some() {
            return None;
        }
        let index = if back {
            (0..self.cursor).rev().find(|&i| exists(&self.panes[i]))
        } else {
            (self.cursor + 1..self.panes.len()).find(|&i| exists(&self.panes[i]))
        }?;
        Some((index, self.panes[index].clone()))
    }

    pub(super) fn requested(&mut self, index: usize) {
        self.pending = Some(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_acknowledged_focus_enters_history() {
        let mut history = FocusHistory::default();
        history.observe(Some("boot"), Some("a"), true);
        history.observe(Some("boot"), Some("b"), false);
        assert!(history.destination(true, |_| true).is_none());
        history.observe(Some("boot"), Some("b"), true);
        assert_eq!(history.destination(true, |_| true), Some((0, "a".into())));
        history.requested(0);
        assert!(history.destination(true, |_| true).is_none());
        history.observe(Some("boot"), Some("a"), false);
        history.observe(Some("boot"), Some("a"), true);
        assert_eq!(history.destination(false, |_| true), Some((1, "b".into())));
    }

    #[test]
    fn new_focus_truncates_forward_history_and_skips_closed_panes() {
        let mut history = FocusHistory::default();
        for pane in ["a", "b", "c"] {
            history.observe(Some("boot"), Some(pane), true);
        }
        assert_eq!(
            history.destination(true, |id| id != "b"),
            Some((0, "a".into()))
        );
        history.requested(0);
        history.observe(Some("boot"), Some("a"), true);
        history.observe(Some("boot"), Some("d"), true);
        assert!(history.destination(false, |_| true).is_none());
        assert_eq!(history.destination(true, |_| true), Some((0, "a".into())));
    }

    #[test]
    fn reconnect_discards_pending_history_and_bounds_growth() {
        let mut history = FocusHistory::default();
        for index in 0..100 {
            history.observe(Some("old"), Some(&index.to_string()), true);
        }
        assert_eq!(history.panes.len(), 64);
        history.requested(0);
        history.observe(None, None, false);
        history.observe(Some("new"), Some("99"), true);
        assert!(history.destination(true, |_| true).is_none());
    }
}
