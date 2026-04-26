use crate::reviewer::{DiffSource, FileEntry};
use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use similar::{ChangeTag, TextDiff};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffBody {
    Lines(Vec<DiffLine>),
    Placeholder(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Insert,
    Delete,
    Hunk,
    Metadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLineMetadata {
    Added {
        mode: String,
    },
    Deleted {
        mode: String,
    },
    ModeChanged {
        old_mode: String,
        new_mode: String,
    },
    Renamed {
        from: String,
        to: String,
        old_mode: String,
        new_mode: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
    pub metadata: Option<DiffLineMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPayload {
    pub title: String,
    pub jump_targets: Vec<usize>,
    pub body: DiffBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FullFilePayload {
    pub lines: Vec<String>,
    pub jump_targets: Vec<usize>,
    pub changed_lines: BTreeSet<usize>,
    pub deleted_before: BTreeMap<usize, Vec<String>>,
}

pub fn load_diff(project_root: &Path, file: &FileEntry) -> Result<DiffPayload> {
    match &file.diff_source {
        DiffSource::CommitBacked { commit, repo } => {
            load_commit_backed_diff(project_root, commit, repo.as_deref(), &file.path)
        }
        DiffSource::WorkingTree {
            repo,
            staged,
            unstaged,
            untracked,
        } => load_working_tree_diff(
            project_root,
            repo.as_deref(),
            &file.path,
            *staged,
            *unstaged,
            *untracked,
        ),
        DiffSource::NonCommitFallback { hint } => Ok(DiffPayload {
            title: file.path.clone(),
            jump_targets: Vec::new(),
            body: DiffBody::Placeholder(format!(
                "No commit-backed diff is available for this file.\n{}",
                hint
            )),
        }),
    }
}

pub fn load_full_file(project_root: &Path, file: &FileEntry) -> Result<String> {
    match &file.diff_source {
        DiffSource::CommitBacked { commit, repo } => {
            let repo_root = repo_root(project_root, repo.as_deref());
            let repo_relative_path = repo_relative_path(&file.path, repo.as_deref());
            load_file_at_revision_required(&repo_root, commit, repo_relative_path)
        }
        DiffSource::WorkingTree { repo, .. } => {
            let repo_root = repo_root(project_root, repo.as_deref());
            let repo_relative_path = repo_relative_path(&file.path, repo.as_deref());
            let path = repo_root.join(repo_relative_path);
            fs::read_to_string(&path)
                .with_context(|| format!("failed to read working tree file {}", path.display()))
        }
        DiffSource::NonCommitFallback { .. } => {
            let path = project_root.join(&file.path);
            fs::read_to_string(&path)
                .with_context(|| format!("failed to read full file {}", path.display()))
        }
    }
}

pub fn load_full_file_payload(project_root: &Path, file: &FileEntry) -> Result<FullFilePayload> {
    let content = load_full_file(project_root, file)?;
    let lines = content.lines().map(ToString::to_string).collect::<Vec<_>>();
    let (jump_targets, changed_lines, deleted_before) = match &file.diff_source {
        DiffSource::CommitBacked { commit, repo } => {
            let repo_root = repo_root(project_root, repo.as_deref());
            let repo_relative_path = repo_relative_path(&file.path, repo.as_deref());
            let (before, after) = load_file_pair_at_commit(&repo_root, commit, repo_relative_path)?;
            structured_overlay(&structured_diff_lines(
                repo_relative_path,
                &before,
                &after,
                commit,
            ))
        }
        DiffSource::WorkingTree { repo, .. } => {
            let repo_root = repo_root(project_root, repo.as_deref());
            let repo_relative_path = repo_relative_path(&file.path, repo.as_deref());
            let before = load_file_at_revision_optional(&repo_root, "HEAD", repo_relative_path)?
                .unwrap_or_default();
            structured_overlay(&structured_diff_lines(
                repo_relative_path,
                &before,
                &content,
                "worktree",
            ))
        }
        DiffSource::NonCommitFallback { .. } => (Vec::new(), BTreeSet::new(), BTreeMap::new()),
    };
    let (jump_targets, deleted_before) =
        normalize_full_file_overlay(jump_targets, deleted_before, lines.len());

    Ok(FullFilePayload {
        lines,
        jump_targets,
        changed_lines,
        deleted_before,
    })
}

fn load_working_tree_diff(
    project_root: &Path,
    repo: Option<&str>,
    path: &str,
    staged: bool,
    unstaged: bool,
    untracked: bool,
) -> Result<DiffPayload> {
    let repo_root = repo_root(project_root, repo);
    let repo_relative_path = repo_relative_path(path, repo);
    let title = path.to_string();
    if untracked {
        let full_path = repo_root.join(repo_relative_path);
        let content = fs::read_to_string(&full_path).unwrap_or_default();
        let body = render_untracked_file_diff(&content);
        return Ok(DiffPayload {
            title,
            jump_targets: Vec::new(),
            body: DiffBody::Lines(body),
        });
    }

    let mut lines = Vec::new();
    if staged {
        if let Ok(mut diff) = load_working_tree_raw_diff(&repo_root, repo_relative_path, true)
            && !diff.is_empty()
        {
            if !lines.is_empty() {
                lines.push(separator_line());
            }
            lines.push(section_line("staged"));
            lines.append(&mut diff);
        }
    }
    if unstaged {
        if let Ok(mut diff) = load_working_tree_raw_diff(&repo_root, repo_relative_path, false)
            && !diff.is_empty()
        {
            if !lines.is_empty() {
                lines.push(separator_line());
            }
            lines.push(section_line("unstaged"));
            lines.append(&mut diff);
        }
    }
    let body = if lines.is_empty() {
        DiffBody::Placeholder("No working tree diff is available for this file.".to_string())
    } else {
        DiffBody::Lines(lines)
    };
    Ok(DiffPayload {
        title,
        jump_targets: Vec::new(),
        body,
    })
}

/// Builds the reviewer diff for one staged or unstaged working-tree section.
fn load_working_tree_raw_diff(repo_root: &Path, path: &str, staged: bool) -> Result<Vec<DiffLine>> {
    let label = if staged { "index" } else { "worktree" };
    let (before, after) = if staged {
        let before = load_file_at_revision_optional(repo_root, "HEAD", path)?.unwrap_or_default();
        let after = load_file_at_index_optional(repo_root, path)?.unwrap_or_default();
        (before, after)
    } else {
        // Trigger: a path can be staged and then edited again, producing `AM`
        // or `MM` status.
        // Why: unstaged diff must start from the index, not HEAD, otherwise
        // the normal split view repeats the staged part for new files.
        // Prevents: duplicated additions and misleading unstaged hunks.
        let before = match load_file_at_index_optional(repo_root, path)? {
            Some(content) => content,
            None => load_file_at_revision_optional(repo_root, "HEAD", path)?.unwrap_or_default(),
        };
        let after = fs::read_to_string(repo_root.join(path)).unwrap_or_default();
        (before, after)
    };
    Ok(structured_diff_lines(path, &before, &after, label))
}

fn render_untracked_file_diff(content: &str) -> Vec<DiffLine> {
    let mut lines = vec![separator_line()];
    for (index, line) in content.lines().enumerate() {
        lines.push(DiffLine {
            kind: DiffLineKind::Insert,
            old_line: None,
            new_line: Some(index + 1),
            text: line.to_string(),
            metadata: None,
        });
    }
    if content.is_empty() {
        lines.push(DiffLine {
            kind: DiffLineKind::Insert,
            old_line: None,
            new_line: Some(1),
            text: String::new(),
            metadata: None,
        });
    }
    lines
}

fn load_commit_backed_diff(
    project_root: &Path,
    commit: &str,
    repo: Option<&str>,
    path: &str,
) -> Result<DiffPayload> {
    let debug_perf = env::var_os("GSDV_DEBUG_PERF").is_some();
    let started = Instant::now();
    let repo_root = repo_root(project_root, repo);
    let repo_relative_path = repo_relative_path(path, repo);
    let title = path.to_string();

    let lines = match load_structured_diff(&repo_root, commit, repo_relative_path) {
        Ok(Some(lines)) => lines,
        Ok(None) => match load_metadata_diff_at_commit(&repo_root, commit, repo_relative_path)? {
            Some(lines) => lines,
            None => {
                return Ok(DiffPayload {
                    title,
                    jump_targets: Vec::new(),
                    body: DiffBody::Placeholder(format!(
                        "No commit-backed diff is available for `{repo_relative_path}` from `{commit}`."
                    )),
                });
            }
        },
        Err(error) => {
            return Ok(DiffPayload {
                title,
                jump_targets: Vec::new(),
                body: DiffBody::Error(error.to_string()),
            });
        }
    };

    let jump_targets = lines
        .iter()
        .filter(|line| line.kind == DiffLineKind::Hunk)
        .filter_map(|line| line.old_line)
        .collect::<Vec<_>>();

    let payload = DiffPayload {
        title,
        jump_targets,
        body: DiffBody::Lines(lines),
    };
    if debug_perf {
        eprintln!(
            "[perf] load_commit_backed_diff path={} commit={} total_ms={}",
            path,
            commit,
            started.elapsed().as_millis()
        );
    }
    Ok(payload)
}

fn load_structured_diff(
    repo_root: &Path,
    commit: &str,
    path: &str,
) -> Result<Option<Vec<DiffLine>>> {
    let Ok((before, after)) = load_file_pair_at_commit(repo_root, commit, path) else {
        return Ok(None);
    };
    let lines = structured_diff_lines(path, &before, &after, commit);
    if lines.is_empty() {
        return Ok(None);
    }
    Ok(Some(lines))
}

fn load_file_at_revision_required(repo_root: &Path, revision: &str, path: &str) -> Result<String> {
    load_file_at_revision_optional(repo_root, revision, path)?
        .ok_or_else(|| anyhow::anyhow!("Unable to load `{path}` at revision `{revision}`."))
}

fn load_file_pair_at_commit(
    repo_root: &Path,
    commit: &str,
    path: &str,
) -> Result<(String, String)> {
    let repo = gix::open(repo_root)
        .with_context(|| format!("failed to open git repository {}", repo_root.display()))?;
    let id = repo
        .rev_parse_single(commit)
        .with_context(|| format!("failed to resolve commit `{commit}`"))?;
    let object = id.object()?;
    let commit = object
        .try_into_commit()
        .map_err(|error| anyhow::anyhow!("expected commit while loading `{path}`: {error}"))?;
    let after = load_file_from_commit(&commit, path)?.unwrap_or_default();
    let before = commit
        .parent_ids()
        .next()
        .and_then(|parent| parent.object().ok())
        .and_then(|object| object.try_into_commit().ok())
        .map(|parent| load_file_from_commit(&parent, path))
        .transpose()?
        .flatten()
        .unwrap_or_default();
    Ok((before, after))
}

fn load_file_at_revision_optional(
    repo_root: &Path,
    revision: &str,
    path: &str,
) -> Result<Option<String>> {
    let repo = gix::open(repo_root)
        .with_context(|| format!("failed to open git repository {}", repo_root.display()))?;
    let id = match repo.rev_parse_single(revision) {
        Ok(id) => id,
        Err(_) => return Ok(None),
    };
    let object = match id.object() {
        Ok(object) => object,
        Err(_) => return Ok(None),
    };
    let commit = match object.try_into_commit() {
        Ok(commit) => commit,
        Err(_) => return Ok(None),
    };
    load_file_from_commit(&commit, path)
}

/// Loads the current index blob for a repository-relative path.
fn load_file_at_index_optional(repo_root: &Path, path: &str) -> Result<Option<String>> {
    let repo = gix::open(repo_root)
        .with_context(|| format!("failed to open git repository {}", repo_root.display()))?;
    let index = match repo.try_index()? {
        Some(index) => index,
        None => return Ok(None),
    };
    let entry = match index.entry_by_path(path.as_bytes().as_bstr()) {
        Some(entry) => entry,
        None => return Ok(None),
    };
    let blob = match repo.find_object(entry.id)?.try_into_blob() {
        Ok(blob) => blob,
        Err(_) => return Ok(None),
    };
    Ok(Some(String::from_utf8_lossy(&blob.data).to_string()))
}

fn load_file_from_commit(commit: &gix::Commit<'_>, path: &str) -> Result<Option<String>> {
    let tree = commit.tree()?;
    let entry = match tree.lookup_entry_by_path(path)? {
        Some(entry) => entry,
        None => return Ok(None),
    };
    let blob = match entry.object()?.try_into_blob() {
        Ok(blob) => blob,
        Err(_) => return Ok(None),
    };
    Ok(Some(String::from_utf8_lossy(&blob.data).to_string()))
}

fn load_metadata_diff_at_commit(
    repo_root: &Path,
    commit: &str,
    path: &str,
) -> Result<Option<Vec<DiffLine>>> {
    let repo = gix::open(repo_root)
        .with_context(|| format!("failed to open git repository {}", repo_root.display()))?;
    let Ok(id) = repo.rev_parse_single(commit) else {
        return Ok(None);
    };
    let Ok(object) = id.object() else {
        return Ok(None);
    };
    let Ok(commit) = object.try_into_commit() else {
        return Ok(None);
    };
    let new_tree = commit.tree()?;
    let old_tree = commit
        .parent_ids()
        .next()
        .and_then(|parent| parent.object().ok())
        .and_then(|object| object.try_into_commit().ok())
        .and_then(|parent| parent.tree().ok());

    for change in repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)? {
        let Some(lines) = metadata_lines_for_change(&change, path) else {
            continue;
        };
        if !lines.is_empty() {
            return Ok(Some(lines));
        }
    }
    Ok(None)
}

fn metadata_lines_for_change(
    change: &gix::object::tree::diff::ChangeDetached,
    path: &str,
) -> Option<Vec<DiffLine>> {
    match change {
        gix::object::tree::diff::ChangeDetached::Addition {
            entry_mode,
            location,
            ..
        } if location == path => Some(vec![metadata_line(DiffLineMetadata::Added {
            mode: format!("{entry_mode:?}"),
        })]),
        gix::object::tree::diff::ChangeDetached::Deletion {
            entry_mode,
            location,
            ..
        } if location == path => Some(vec![metadata_line(DiffLineMetadata::Deleted {
            mode: format!("{entry_mode:?}"),
        })]),
        gix::object::tree::diff::ChangeDetached::Modification {
            previous_entry_mode,
            entry_mode,
            location,
            ..
        } if location == path && previous_entry_mode != entry_mode => {
            Some(vec![metadata_line(DiffLineMetadata::ModeChanged {
                old_mode: format!("{previous_entry_mode:?}"),
                new_mode: format!("{entry_mode:?}"),
            })])
        }
        gix::object::tree::diff::ChangeDetached::Rewrite {
            source_location,
            source_entry_mode,
            entry_mode,
            location,
            ..
        } if location == path || source_location == path => {
            Some(vec![metadata_line(DiffLineMetadata::Renamed {
                from: source_location.to_string(),
                to: location.to_string(),
                old_mode: format!("{source_entry_mode:?}"),
                new_mode: format!("{entry_mode:?}"),
            })])
        }
        _ => None,
    }
}

fn metadata_line(metadata: DiffLineMetadata) -> DiffLine {
    DiffLine {
        kind: DiffLineKind::Metadata,
        old_line: None,
        new_line: None,
        text: String::new(),
        metadata: Some(metadata),
    }
}

fn structured_diff_lines(path: &str, before: &str, after: &str, label: &str) -> Vec<DiffLine> {
    let before = normalize_diff_text(before);
    let after = normalize_diff_text(after);
    if before == after {
        return Vec::new();
    }

    let diff = TextDiff::from_lines(&before, &after);
    let mut lines = vec![file_header_line(path), separator_line()];
    for group in diff.grouped_ops(3) {
        let changes = group
            .iter()
            .flat_map(|op| diff.iter_changes(op))
            .collect::<Vec<_>>();
        lines.push(hunk_line(&changes, label));
        lines.extend(changes.into_iter().map(|change| {
            let kind = match change.tag() {
                ChangeTag::Equal => DiffLineKind::Context,
                ChangeTag::Delete => DiffLineKind::Delete,
                ChangeTag::Insert => DiffLineKind::Insert,
            };
            DiffLine {
                kind,
                old_line: change.old_index().map(|index| index + 1),
                new_line: change.new_index().map(|index| index + 1),
                text: diff_line_text(change.value()),
                metadata: None,
            }
        }));
    }
    lines
}

fn file_header_line(path: &str) -> DiffLine {
    DiffLine {
        kind: DiffLineKind::Context,
        old_line: None,
        new_line: None,
        text: path.to_string(),
        metadata: None,
    }
}

fn hunk_line(changes: &[similar::Change<&str>], label: &str) -> DiffLine {
    let old_numbers = changes
        .iter()
        .filter_map(|change| change.old_index().map(|index| index + 1))
        .collect::<Vec<_>>();
    let new_numbers = changes
        .iter()
        .filter_map(|change| change.new_index().map(|index| index + 1))
        .collect::<Vec<_>>();
    let old_start = old_numbers.first().copied().unwrap_or(0);
    let old_count = old_numbers.len();
    let new_start = new_numbers.first().copied().unwrap_or(0);
    let new_count = new_numbers.len();

    DiffLine {
        kind: DiffLineKind::Hunk,
        old_line: Some(old_start),
        new_line: Some(new_start),
        text: format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@ {label}"),
        metadata: None,
    }
}

fn diff_line_text(value: &str) -> String {
    value.trim_end_matches(['\r', '\n']).to_string()
}

fn normalize_diff_text(value: &str) -> Cow<'_, str> {
    if value.contains('\r') {
        Cow::Owned(value.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(value)
    }
}

fn structured_overlay(
    lines: &[DiffLine],
) -> (Vec<usize>, BTreeSet<usize>, BTreeMap<usize, Vec<String>>) {
    let mut jump_targets = Vec::new();
    let mut changed = BTreeSet::new();
    let mut deleted_before: BTreeMap<usize, Vec<String>> = BTreeMap::new();

    for line in lines {
        match line.kind {
            DiffLineKind::Hunk => {
                if let Some(start) = line.new_line {
                    jump_targets.push(start);
                }
            }
            DiffLineKind::Insert => {
                if let Some(line_number) = line.new_line {
                    changed.insert(line_number);
                }
            }
            DiffLineKind::Delete => {
                let anchor = line
                    .new_line
                    .or_else(|| line.old_line.map(|line| line + 1))
                    .unwrap_or(1);
                deleted_before
                    .entry(anchor)
                    .or_default()
                    .push(line.text.clone());
            }
            DiffLineKind::Context | DiffLineKind::Metadata => {}
        }
    }

    jump_targets.sort_unstable();
    jump_targets.dedup();
    (jump_targets, changed, deleted_before)
}

/// Normalizes full-file overlay anchors to rows that the full renderer can draw.
fn normalize_full_file_overlay(
    mut jump_targets: Vec<usize>,
    deleted_before: BTreeMap<usize, Vec<String>>,
    line_count: usize,
) -> (Vec<usize>, BTreeMap<usize, Vec<String>>) {
    let trailing_anchor = line_count.saturating_add(1).max(1);
    for target in &mut jump_targets {
        *target = (*target).clamp(1, trailing_anchor);
    }
    jump_targets.sort_unstable();
    jump_targets.dedup();

    let mut normalized_deleted = BTreeMap::new();
    for (anchor, lines) in deleted_before {
        let anchor = anchor.clamp(1, trailing_anchor);
        normalized_deleted
            .entry(anchor)
            .or_insert_with(Vec::new)
            .extend(lines);
    }
    (jump_targets, normalized_deleted)
}

fn section_line(label: &str) -> DiffLine {
    DiffLine {
        kind: DiffLineKind::Context,
        old_line: None,
        new_line: None,
        text: label.to_string(),
        metadata: None,
    }
}

fn separator_line() -> DiffLine {
    DiffLine {
        kind: DiffLineKind::Context,
        old_line: None,
        new_line: None,
        text: "─".repeat(80),
        metadata: None,
    }
}

fn repo_root(project_root: &Path, repo: Option<&str>) -> PathBuf {
    match repo {
        Some(repo_name) => project_root.join(repo_name),
        None => project_root.to_path_buf(),
    }
}

fn repo_relative_path<'a>(path: &'a str, repo: Option<&str>) -> &'a str {
    match repo {
        Some(repo_name) => path
            .strip_prefix(repo_name)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or(path),
        None => path,
    }
}

#[cfg(test)]
#[path = "diff_test.rs"]
mod diff_test;
