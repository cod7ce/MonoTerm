// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Fabien Penso
// Worktree grouping adapted from Herdr UI at 38ea4248d2c64879f02053b419bdd5386eebce10.
use herdr_protocol::{AgentStatus, ClientShellWorkspace};
use std::collections::{HashMap, HashSet};

pub(super) struct Entry<'a> {
    pub workspace: &'a ClientShellWorkspace,
    pub child: bool,
    pub last_child: bool,
    pub group: Option<&'a str>,
    pub status: AgentStatus,
}

pub(super) fn entries<'a>(
    workspaces: &'a [ClientShellWorkspace],
    collapsed: &HashSet<String>,
) -> Vec<Entry<'a>> {
    let mut groups = HashMap::<&str, (Vec<usize>, Vec<usize>)>::new();
    for (index, workspace) in workspaces.iter().enumerate() {
        if let Some(tree) = &workspace.worktree {
            let (parents, children) = groups.entry(&tree.key).or_default();
            if tree.is_linked_worktree {
                children.push(index);
            } else {
                parents.push(index);
            }
        }
    }
    groups.retain(|_, (parents, children)| !parents.is_empty() && !children.is_empty());
    let mut emitted = HashSet::new();
    let mut entries = Vec::with_capacity(workspaces.len());
    for workspace in workspaces {
        let group = workspace.worktree.as_ref().and_then(|tree| {
            groups
                .get(tree.key.as_str())
                .map(|members| (tree.key.as_str(), members))
        });
        let Some((key, (parents, children))) = group else {
            entries.push(Entry {
                workspace,
                child: false,
                last_child: false,
                group: None,
                status: workspace.agent_status,
            });
            continue;
        };
        // A group occupies its earliest member's position in daemon order.
        if !emitted.insert(key) {
            continue;
        }
        let folded = collapsed.contains(key);
        let status = parents
            .iter()
            .chain(children)
            .map(|&index| workspaces[index].agent_status)
            .max_by_key(|status| match status {
                AgentStatus::Blocked => 4,
                AgentStatus::Done => 3,
                AgentStatus::Working => 2,
                AgentStatus::Idle => 1,
                AgentStatus::Unknown => 0,
            })
            .unwrap_or(AgentStatus::Unknown);
        for &index in parents {
            let workspace = &workspaces[index];
            entries.push(Entry {
                workspace,
                child: false,
                last_child: false,
                group: Some(key),
                status: if folded {
                    status
                } else {
                    workspace.agent_status
                },
            });
        }
        if !folded {
            for (position, &index) in children.iter().enumerate() {
                let workspace = &workspaces[index];
                entries.push(Entry {
                    workspace,
                    child: true,
                    last_child: position + 1 == children.len(),
                    group: None,
                    status: workspace.agent_status,
                });
            }
        }
    }
    entries
}

pub(super) fn label(workspace: &ClientShellWorkspace, child: bool) -> &str {
    let branch = (child && !workspace.custom_label)
        .then_some(workspace.branch.as_deref())
        .flatten()
        .map(|branch| branch.strip_prefix("worktree/").unwrap_or(branch));
    [branch, Some(workspace.label.as_str())]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|label| !label.is_empty())
        .unwrap_or("workspace")
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_protocol::ClientShellWorktree;

    fn workspace(id: &str, tree: Option<(&str, bool)>) -> ClientShellWorkspace {
        ClientShellWorkspace {
            workspace_id: id.into(),
            active_tab_id: format!("{id}:t1"),
            new_workspace_cwd: "/unavailable".into(),
            number: 1,
            label: id.into(),
            custom_label: false,
            branch: None,
            git_ahead_behind: None,
            tokens: Vec::new(),
            worktree: tree.map(|(key, linked)| ClientShellWorktree {
                key: key.into(),
                label: "repository".into(),
                is_linked_worktree: linked,
            }),
            focused: false,
            agent_status: AgentStatus::Idle,
        }
    }

    #[test]
    fn groups_at_first_member_with_parents_before_children() {
        let spaces = vec![
            workspace("child-a", Some(("repo", true))),
            workspace("unrelated", None),
            workspace("parent-a", Some(("repo", false))),
            workspace("orphan", Some(("other", true))),
            workspace("parent-b", Some(("repo", false))),
            workspace("child-b", Some(("repo", true))),
        ];
        let rows = entries(&spaces, &HashSet::new());
        assert_eq!(
            rows.iter()
                .map(|row| (
                    row.workspace.workspace_id.as_str(),
                    row.child,
                    row.last_child,
                    row.group
                ))
                .collect::<Vec<_>>(),
            vec![
                ("parent-a", false, false, Some("repo")),
                ("parent-b", false, false, Some("repo")),
                ("child-a", true, false, None),
                ("child-b", true, true, None),
                ("unrelated", false, false, None),
                ("orphan", false, false, None),
            ]
        );
    }

    #[test]
    fn collapse_keeps_all_parents_and_surfaces_hidden_attention() {
        let mut spaces = vec![
            workspace("parent-a", Some(("repo", false))),
            workspace("child", Some(("repo", true))),
            workspace("parent-b", Some(("repo", false))),
        ];
        spaces[1].focused = true;
        spaces[1].agent_status = AgentStatus::Blocked;
        let rows = entries(&spaces, &HashSet::from(["repo".into()]));
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| !row.child && !row.workspace.focused));
        assert!(rows.iter().all(|row| row.status == AgentStatus::Blocked));
        assert!(spaces[1].focused);
        assert_eq!(entries(&spaces, &HashSet::new()).len(), 3);
    }

    #[test]
    fn checkouts_without_both_kinds_stay_top_level() {
        let spaces = vec![
            workspace("main-a", Some(("repo", false))),
            workspace("main-b", Some(("repo", false))),
            workspace("linked-a", Some(("other", true))),
            workspace("linked-b", Some(("other", true))),
        ];
        let rows = entries(&spaces, &HashSet::from(["repo".into(), "other".into()]));
        assert_eq!(rows.len(), spaces.len());
        assert!(rows.iter().all(|row| !row.child && row.group.is_none()));
        assert!(
            rows.iter()
                .zip(&spaces)
                .all(|(row, workspace)| row.workspace == workspace)
        );
    }

    #[test]
    fn child_labels_use_branch_unless_custom_and_fall_back_safely() {
        let mut space = workspace("Project", Some(("repo", true)));
        space.branch = Some("worktree/fix-sidebar".into());
        assert_eq!(label(&space, true), "fix-sidebar");
        assert_eq!(label(&space, false), "Project");
        space.custom_label = true;
        assert_eq!(label(&space, true), "Project");
        space.custom_label = false;
        space.branch = Some(" ".into());
        assert_eq!(label(&space, true), "Project");
        space.label = " ".into();
        assert_eq!(label(&space, true), "workspace");
    }
}
