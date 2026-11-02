//! Pure worktree menu models. Callers validate captured targets against the
//! latest snapshot and correlate responses before sending any daemon request.

use crate::herdr_connection::WorkspaceTarget;
use herdr_client::protocol::{ClientShellSnapshot, ClientShellWorkspace};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

const MAX_ENTRIES: usize = 512;
const MAX_TEXT: usize = 8192;
const INVALID_LIST: &str = "Malformed worktree list; reopen the menu";

#[derive(Clone)]
pub struct WorktreeSource {
    pub target: WorkspaceTarget,
    /// Display form: HEAD, or a branch without its refs/heads/ prefix.
    pub base: String,
    base_ref: String,
    clicked: WorkspaceTarget,
    // A moved main workspace must still address the linked checkout's repo.
    cwd: Option<String>,
}

impl WorktreeSource {
    pub fn new(snapshot: &ClientShellSnapshot, clicked: &WorkspaceTarget) -> Result<Self, String> {
        validate_target(clicked, snapshot)?;
        let mut source = Self {
            target: clicked.clone(),
            base: "HEAD".into(),
            base_ref: "HEAD".into(),
            clicked: clicked.clone(),
            cwd: None,
        };
        let workspace = &clicked.workspace;
        if let Some(tree) = workspace.worktree.as_ref().filter(|t| t.is_linked_worktree) {
            let branch = workspace
                .branch
                .as_deref()
                .ok_or("This checkout has no branch to start a worktree from")?;
            validate_branch(branch)?;
            let candidates = snapshot.workspaces.iter().filter(|w| {
                w.worktree
                    .as_ref()
                    .is_some_and(|t| t.key == tree.key && !t.is_linked_worktree)
            });
            let main = candidates
                .min_by_key(|w| left_checkout(w))
                .ok_or("Open this repository's main checkout to create a worktree")?;
            source.target = WorkspaceTarget::new(snapshot, main);
            source.base = branch.to_owned();
            // A full ref keeps a tag of the same name from shadowing the branch.
            source.base_ref = format!("refs/heads/{branch}");
            if left_checkout(main) {
                source.cwd = checkout_root(&tree.key).map(str::to_owned);
            }
        } else if !can_open(clicked) {
            return Err("This workspace is not a Git repository".into());
        }
        Ok(source)
    }

    pub fn validate(&self, snapshot: &ClientShellSnapshot) -> Result<(), String> {
        validate_target(&self.clicked, snapshot)?;
        validate_target(&self.target, snapshot)
    }

    pub fn list_params(&self) -> Value {
        match &self.cwd {
            Some(cwd) => json!({"cwd": cwd, "trust_repository": false}),
            None => list_params(&self.target),
        }
    }

    /// Blank input delegates naming to the daemon. Validate the current
    /// snapshot separately, immediately before queueing this request.
    pub fn create_params(&self, branch: &str) -> Result<Value, String> {
        let mut params = self.list_params();
        params["base"] = self.base_ref.clone().into();
        params["focus"] = true.into();
        let branch = branch.trim();
        if !branch.is_empty() {
            validate_branch(branch)?;
            params["branch"] = branch.into();
        }
        Ok(params)
    }
}

fn validate_target(target: &WorkspaceTarget, snapshot: &ClientShellSnapshot) -> Result<(), String> {
    target.validate(snapshot)?;
    if !snapshot.workspaces.iter().any(|workspace| {
        workspace.workspace_id == target.workspace.workspace_id
            && workspace.branch == target.workspace.branch
    }) {
        return Err("Herdr workspace branch changed; reopen its menu".into());
    }
    Ok(())
}

pub fn can_delete(target: &WorkspaceTarget) -> bool {
    let workspace = &target.workspace;
    workspace
        .worktree
        .as_ref()
        .is_some_and(|tree| tree.is_linked_worktree)
}

pub fn can_open(target: &WorkspaceTarget) -> bool {
    let workspace = &target.workspace;
    !can_delete(target)
        && (workspace.branch.is_some()
            || (workspace.worktree.is_some() && !left_checkout(workspace)))
}

/// Explicit source for main opening or linked deletion lookup.
pub fn list_params(target: &WorkspaceTarget) -> Value {
    if left_checkout(&target.workspace) {
        json!({"cwd": target.workspace.new_workspace_cwd, "trust_repository": false})
    } else {
        json!({"workspace_id": target.workspace.workspace_id, "trust_repository": false})
    }
}

/// Select a path from a correlated, parsed list and validate the target before
/// queueing; this function only builds parameters.
pub fn open_params(target: &WorkspaceTarget, path: &str) -> Value {
    let mut params = list_params(target);
    params["path"] = path.into();
    params["focus"] = true.into();
    params
}

fn checkout_root(key: &str) -> Option<&str> {
    key.strip_suffix("/.git")
        .or_else(|| key.strip_suffix("\\.git"))
        .filter(|root| !root.is_empty())
}

// Endpoint paths can be Windows paths on a Unix client. Unknown key formats
// cannot establish that the shell left its checkout, matching upstream.
fn left_checkout(workspace: &ClientShellWorkspace) -> bool {
    let Some(tree) = workspace
        .worktree
        .as_ref()
        .filter(|t| !t.is_linked_worktree)
    else {
        return false;
    };
    let Some(root) = checkout_root(&tree.key) else {
        return false;
    };
    if workspace.new_workspace_cwd.is_empty() {
        return false;
    }
    let normalize = |path: &str| {
        let path = path.trim_end_matches(['/', '\\']);
        if root.starts_with('/') {
            path.to_owned()
        } else {
            path.replace('\\', "/").to_lowercase()
        }
    };
    let (root, cwd) = (normalize(root), normalize(&workspace.new_workspace_cwd));
    cwd != root
        && !cwd
            .strip_prefix(&root)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn validate_branch(branch: &str) -> Result<(), String> {
    if branch.is_empty()
        || branch.len() > MAX_TEXT
        || matches!(branch, "@" | "HEAD")
        || branch.starts_with('-')
        || branch.ends_with('.')
        || branch.contains("..")
        || branch.contains("@{")
        || branch
            .bytes()
            .any(|byte| byte <= b' ' || byte == 0x7f || b"~^:?*[\\".contains(&byte))
        || branch
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.') || part.ends_with(".lock"))
    {
        return Err("Invalid Git branch name".into());
    }
    Ok(())
}

#[derive(Clone)]
pub struct WorktreeList {
    pub repo_key: String,
    pub entries: Vec<WorktreeEntry>,
}

#[derive(Clone, Deserialize)]
pub struct WorktreeEntry {
    pub path: String,
    pub branch: Option<String>,
    pub label: String,
    pub open_workspace_id: Option<String>,
    is_linked_worktree: bool,
    is_bare: bool,
    is_prunable: bool,
    #[serde(rename = "is_detached")]
    _is_detached: bool,
}

impl WorktreeList {
    pub fn parse(response: &Value) -> Result<Self, String> {
        let result = &response["result"];
        let rows = result["worktrees"]
            .as_array()
            .filter(|rows| {
                result["type"] == "worktree_list"
                    && rows.len() <= MAX_ENTRIES
                    && response.get("error").is_none_or(Value::is_null)
            })
            .ok_or(INVALID_LIST)?;
        for field in ["repo_key", "repo_name", "repo_root", "source_checkout_path"] {
            bounded_text(&result["source"][field], false, false)?;
        }
        bounded_text(&result["source"]["source_workspace_id"], true, false)?;
        let mut entries = Vec::with_capacity(rows.len());
        let mut paths = HashSet::new();
        let mut workspaces = HashSet::new();
        for row in rows {
            for (field, optional, empty) in [
                ("path", false, false),
                ("label", false, true),
                ("branch", true, true),
                ("open_workspace_id", true, false),
            ] {
                bounded_text(&row[field], optional, empty)?;
            }
            let entry = WorktreeEntry::deserialize(row).map_err(|_| INVALID_LIST)?;
            // Filter only after checking identities: discarded rows must not
            // conceal ambiguity in a destructive action's response.
            if !paths.insert(entry.path.clone())
                || entry
                    .open_workspace_id
                    .as_ref()
                    .is_some_and(|id| !workspaces.insert(id.clone()))
            {
                return Err(INVALID_LIST.into());
            }
            if !entry.is_bare && !entry.is_prunable {
                entries.push(entry);
            }
        }
        Ok(Self {
            repo_key: result["source"]["repo_key"]
                .as_str()
                .ok_or(INVALID_LIST)?
                .into(),
            entries,
        })
    }

    pub fn deletion_path(&self, target: &WorkspaceTarget) -> Result<String, String> {
        let tree = target.workspace.worktree.as_ref();
        if !tree.is_some_and(|tree| tree.is_linked_worktree && tree.key == self.repo_key) {
            return Err("Worktree repository changed; reopen the delete confirmation".into());
        }
        let id = target.workspace.workspace_id.as_str();
        let mut matches = self
            .entries
            .iter()
            .filter(|e| e.open_workspace_id.as_deref() == Some(id));
        if let Some(entry) = matches.next()
            && matches.next().is_none()
            && entry.is_linked_worktree
            && !entry.is_bare
            && !entry.is_prunable
            && !entry.path.is_empty()
        {
            return Ok(entry.path.clone());
        }
        Err(
            "Daemon did not identify a unique linked checkout; reopen the delete confirmation"
                .into(),
        )
    }
}

fn bounded_text(value: &Value, optional: bool, empty: bool) -> Result<(), String> {
    if (optional && value.is_null())
        || value.as_str().is_some_and(|text| {
            (empty || !text.is_empty()) && text.len() <= MAX_TEXT && !text.contains('\0')
        })
    {
        Ok(())
    } else {
        Err(INVALID_LIST.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> ClientShellSnapshot {
        let mut s: ClientShellSnapshot = serde_json::from_str(include_str!(
            "../vendor/herdr/herdr-protocol/tests/fixtures/endpoint-snapshot-v1.json"
        ))
        .unwrap();
        let main = &mut s.workspaces[0];
        main.workspace_id = "main".into();
        let tree = main.worktree.as_mut().unwrap();
        tree.key = "/repo/.git".into();
        tree.is_linked_worktree = false;
        let mut linked = main.clone();
        linked.workspace_id = "linked".into();
        linked.branch = Some("feature/task".into());
        linked.worktree.as_mut().unwrap().is_linked_worktree = true;
        s.workspaces.push(linked);
        s
    }
    fn target(s: &ClientShellSnapshot, index: usize) -> WorkspaceTarget {
        WorkspaceTarget::new(s, &s.workspaces[index])
    }
    fn source(s: &ClientShellSnapshot, index: usize) -> WorktreeSource {
        WorktreeSource::new(s, &target(s, index)).unwrap()
    }
    fn response() -> Value {
        json!({"result": {"type": "worktree_list", "source": {
            "repo_key": "/repo/.git", "repo_name": "repo", "repo_root": "/repo", "source_checkout_path": "/repo", "source_workspace_id": null
        }, "worktrees": [{"path": "/trees/feature", "branch": "feature/task", "label": "feature",
            "open_workspace_id": "linked", "is_linked_worktree": true, "is_bare": false, "is_prunable": false, "is_detached": false}]}})
    }
    #[test]
    fn requests_capture_explicit_sources_and_literal_branches() {
        let mut s = snapshot();
        let main = target(&s, 0);
        let linked = target(&s, 1);
        let captured = source(&s, 1);
        assert_eq!(captured.base, "feature/task", "the dialog names the branch");
        s.focused_workspace_id = Some("unrelated".into());
        assert!(captured.validate(&s).is_ok());
        let requests = json!([
            captured.create_params(" topic/new ").unwrap(),
            source(&s, 0).create_params(" ").unwrap(),
            list_params(&linked),
            open_params(&main, "/trees/feature")
        ]);
        assert_eq!(
            requests,
            json!([
                {"workspace_id":"main", "base":"refs/heads/feature/task", "branch":"topic/new", "focus":true, "trust_repository":false},
                {"workspace_id":"main", "base":"HEAD", "focus":true, "trust_repository":false},
                {"workspace_id":"linked", "trust_repository":false},
                {"workspace_id":"main", "path":"/trees/feature", "focus":true, "trust_repository":false}
            ])
        );
        assert!(can_open(&main) && !can_delete(&main));
        assert!(!can_open(&linked) && can_delete(&linked));
        for branch in [
            "HEAD", "@", "-option", "a..b", "a@{b}", "a b", "a\0", "a~", "a^", "a:", "a?", "a*",
            "a[", "a\\b", "/a", "a/", "a//b", ".a", "a/.b", "a.lock", "a.",
        ] {
            assert!(captured.create_params(branch).is_err(), "{branch:?}");
        }
        assert!(captured.create_params(&"x".repeat(MAX_TEXT + 1)).is_err());
        assert!(captured.create_params("功能/修复").is_ok());
    }
    #[test]
    fn creation_rejects_changed_original_or_resolved_target() {
        let s = snapshot();
        let captured = source(&s, 1);
        for index in 0..2 {
            for field in 0..4 {
                let mut changed = s.clone();
                let workspace = &mut changed.workspaces[index];
                match field {
                    0 => workspace.branch = Some("switched".into()),
                    1 => workspace.new_workspace_cwd = "/elsewhere".into(),
                    2 => workspace.worktree.as_mut().unwrap().key = "/other/.git".into(),
                    _ => workspace.workspace_id = "replaced".into(),
                }
                assert!(captured.validate(&changed).is_err());
            }
        }
        let mut changed = s.clone();
        changed.boot_id = "restarted".into();
        assert!(captured.validate(&changed).is_err());
        assert!(WorktreeSource::new(&changed, &target(&s, 1)).is_err());
        changed = s.clone();
        changed.workspaces.remove(0);
        assert!(WorktreeSource::new(&changed, &target(&s, 1)).is_err());
        changed.workspaces[0].branch = None;
        assert!(WorktreeSource::new(&changed, &target(&changed, 0)).is_err());
    }
    #[test]
    fn moved_main_uses_cwd_but_linked_creation_uses_original_repository() {
        let mut s = snapshot();
        s.workspaces[0].new_workspace_cwd = "/repo-other".into();
        assert_eq!(
            source(&s, 0).list_params(),
            json!({"cwd": "/repo-other", "trust_repository": false})
        );
        assert_eq!(
            source(&s, 1).list_params(),
            json!({"cwd": "/repo", "trust_repository": false})
        );
        assert!(source(&s, 1).validate(&s).is_ok());
        s.workspaces[0].branch = None;
        assert!(!can_open(&target(&s, 0)));
        assert!(WorktreeSource::new(&s, &target(&s, 0)).is_err());
        let mut home = s.workspaces[0].clone();
        home.workspace_id = "home".into();
        home.new_workspace_cwd = "/repo/subdir".into();
        s.workspaces.push(home);
        assert_eq!(source(&s, 1).list_params()["workspace_id"], "home");
        let main = &mut s.workspaces[0];
        main.worktree.as_mut().unwrap().key = "C:\\Repo\\.git".into();
        main.new_workspace_cwd = "c:/REPO/subdir/".into();
        assert!(!left_checkout(main));
        main.new_workspace_cwd = "c:/repo-other".into();
        assert!(left_checkout(main));
    }
    #[test]
    fn listing_checks_schema_limits_and_duplicate_identities_before_filtering() {
        let valid = response();
        for (pointer, value) in [
            ("/result/type", json!("other")),
            ("/result/source/repo_key", json!("")),
            ("/result/source/repo_root", Value::Null),
            ("/result/source/source_workspace_id", json!(42)),
            ("/result/worktrees/0/is_detached", Value::Null),
            ("/result/worktrees/0/is_linked_worktree", json!("true")),
            ("/result/worktrees/0/path", json!("")),
            ("/result/worktrees/0/branch", json!(42)),
            ("/result/worktrees/0/label", json!("\0")),
            ("/result/worktrees/0/path", json!("x".repeat(MAX_TEXT + 1))),
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(WorktreeList::parse(&invalid).is_err(), "{pointer}");
        }
        for same_path in [true, false] {
            let mut invalid = valid.clone();
            let mut duplicate = valid["result"]["worktrees"][0].clone();
            let field = if same_path {
                "open_workspace_id"
            } else {
                "path"
            };
            duplicate[field] = "other".into();
            duplicate["is_prunable"] = true.into();
            let rows = invalid["result"]["worktrees"].as_array_mut().unwrap();
            rows.push(duplicate);
            assert!(WorktreeList::parse(&invalid).is_err());
        }
        let mut many = valid.clone();
        many["result"]["worktrees"] = (0..MAX_ENTRIES)
            .map(|i| {
                let mut row = valid["result"]["worktrees"][0].clone();
                row["path"] = format!("/tree/{i}").into();
                row["open_workspace_id"] = Value::Null;
                row
            })
            .collect();
        assert!(WorktreeList::parse(&many).is_ok());
        many["result"]["worktrees"]
            .as_array_mut()
            .unwrap()
            .push(valid["result"]["worktrees"][0].clone());
        assert!(WorktreeList::parse(&many).is_err());
    }
    #[test]
    fn deletion_requires_same_repository_and_one_usable_linked_match() {
        let s = snapshot();
        let target = target(&s, 1);
        let valid = response();
        let mut list = WorktreeList::parse(&valid).unwrap();
        assert_eq!(list.deletion_path(&target).unwrap(), "/trees/feature");
        for (field, value) in [
            ("is_bare", json!(true)),
            ("is_prunable", json!(true)),
            ("is_linked_worktree", json!(false)),
            ("open_workspace_id", json!("other")),
        ] {
            let mut response = valid.clone();
            response["result"]["worktrees"][0][field] = value;
            let list = WorktreeList::parse(&response).unwrap();
            assert!(list.deletion_path(&target).is_err());
            if matches!(field, "is_bare" | "is_prunable") {
                assert!(list.entries.is_empty());
            }
        }
        list.repo_key = "/other/.git".into();
        assert!(list.deletion_path(&target).is_err());
        list.repo_key = "/repo/.git".into();
        list.entries.push(list.entries[0].clone());
        assert!(list.deletion_path(&target).is_err());
    }
}
