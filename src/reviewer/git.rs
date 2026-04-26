use crate::reviewer::{DiffSource, FileEntry};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const UNCOMMITTED_COMMIT_ID: &str = "__UNCOMMITTED__";
pub const GIT_COMMIT_PAGE_SIZE: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRepoReview {
    pub label: String,
    pub repo: Option<String>,
    pub root: PathBuf,
    pub commits: Vec<GitCommitReview>,
    pub commits_loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitReview {
    pub commit: String,
    pub short_hash: String,
    pub author: String,
    pub full_author: String,
    pub display_time: String,
    pub message: String,
    pub files: Vec<GitFileReview>,
    pub files_loaded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitFileReview {
    pub display_path: String,
    pub entry: FileEntry,
}

pub fn load_git_review(project_root: &Path) -> Result<Vec<GitRepoReview>> {
    let mut reviews = discover_git_repo_roots(project_root)?
        .into_iter()
        .map(|repo_root| {
            let (label, repo) = repo_descriptor(project_root, &repo_root);
            let commits = load_git_dirty_commit(&repo_root, repo.as_deref())?
                .into_iter()
                .collect();
            Ok(GitRepoReview {
                label,
                repo,
                root: repo_root,
                commits,
                commits_loaded: false,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    reviews.sort_by(|left, right| left.label.cmp(&right.label));
    Ok(reviews)
}

fn discover_git_repo_roots(project_root: &Path) -> Result<Vec<PathBuf>> {
    let mut repos = BTreeSet::new();
    let mut stack = vec![project_root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if dir == project_root => {
                return Err(error).with_context(|| format!("failed to read {}", dir.display()));
            }
            Err(_) => continue,
        };

        let mut has_git_dir = false;
        let mut children = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == ".git" {
                has_git_dir = true;
                continue;
            }

            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() && !should_skip_dir(&name) {
                children.push(entry.path());
            }
        }

        if has_git_dir {
            repos.insert(dir.clone());
        }
        stack.extend(children);
    }

    Ok(repos.into_iter().collect())
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        "target"
            | "node_modules"
            | "dist"
            | "build"
            | "coverage"
            | ".next"
            | ".nx"
            | ".nuxt"
            | ".svelte-kit"
            | ".turbo"
            | ".git"
            | ".jj"
            | ".svn"
    )
}

fn repo_descriptor(project_root: &Path, repo_root: &Path) -> (String, Option<String>) {
    let Some(relative) = repo_root.strip_prefix(project_root).ok() else {
        return (repo_root.display().to_string(), None);
    };
    if relative.as_os_str().is_empty() {
        return (".".to_string(), None);
    }

    let repo = normalize_slashes(relative);
    (repo.clone(), Some(repo))
}

pub fn load_git_repo_commits(repo_root: &Path) -> Result<Vec<GitCommitReview>> {
    let git_repo = gix::open(repo_root)
        .with_context(|| format!("failed to open git repository {}", repo_root.display()))?;
    let mut commits = Vec::new();

    if let Ok(head) = git_repo.head_id() {
        let walk = git_repo
            .rev_walk([head.detach()])
            .sorting(gix::revision::walk::Sorting::ByCommitTime(
                gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
            ))
            .all()
            .with_context(|| format!("failed to read git history for {}", repo_root.display()))?;
        for info in walk {
            let commit = info?.object()?;
            commits.push(build_commit_review(commit)?);
        }
    }

    Ok(commits)
}

/// Loads one page of commit history for the commit list.
pub fn load_git_repo_commit_page(
    repo_root: &Path,
    skip: usize,
    limit: usize,
) -> Result<Vec<GitCommitReview>> {
    let git_repo = gix::open(repo_root)
        .with_context(|| format!("failed to open git repository {}", repo_root.display()))?;
    let mut commits = Vec::new();

    let Ok(head) = git_repo.head_id() else {
        return Ok(commits);
    };
    let walk = git_repo
        .rev_walk([head.detach()])
        .sorting(gix::revision::walk::Sorting::ByCommitTime(
            gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
        ))
        .all()
        .with_context(|| format!("failed to read git history for {}", repo_root.display()))?;
    for info in walk.skip(skip).take(limit) {
        let commit = info?.object()?;
        commits.push(build_commit_review(commit)?);
    }

    Ok(commits)
}

#[derive(Debug, Clone, Default)]
struct DirtyFileState {
    staged: bool,
    unstaged: bool,
    untracked: bool,
}

pub fn load_git_dirty_commit(
    repo_root: &Path,
    repo: Option<&str>,
) -> Result<Option<GitCommitReview>> {
    let git_repo = gix::open(repo_root)
        .with_context(|| format!("failed to open git repository {}", repo_root.display()))?;
    let mut files = BTreeMap::<String, DirtyFileState>::new();

    for item in git_repo
        .status(gix::progress::Discard)?
        .untracked_files(gix::status::UntrackedFiles::Files)
        .into_iter(Vec::new())
        .with_context(|| format!("failed to read git status for {}", repo_root.display()))?
    {
        let item = item.map_err(|error| anyhow::anyhow!("{error}"))?;
        let path = item.location().to_string();
        if path.is_empty() || path.ends_with('/') || path.contains("/.git/") {
            continue;
        }
        if repo_root.join(&path).join(".git").exists() {
            continue;
        }
        let state = files.entry(path).or_default();
        match item {
            gix::status::Item::TreeIndex(_) => state.staged = true,
            gix::status::Item::IndexWorktree(change) => match change {
                gix::status::index_worktree::Item::DirectoryContents { .. } => {
                    state.untracked = true;
                }
                _ => state.unstaged = true,
            },
        }
    }
    if files.is_empty() {
        return Ok(None);
    }
    let mut dirty_files = Vec::new();
    for (display_path, state) in files {
        let project_relative_path = match repo {
            Some(repo_name) => format!("{repo_name}/{display_path}"),
            None => display_path.clone(),
        };
        dirty_files.push(GitFileReview {
            display_path: display_path.clone(),
            entry: FileEntry {
                path: project_relative_path,
                diff_source: DiffSource::WorkingTree {
                    repo: repo.map(ToString::to_string),
                    staged: state.staged,
                    unstaged: state.unstaged,
                    untracked: state.untracked,
                },
            },
        });
    }
    Ok(Some(GitCommitReview {
        commit: UNCOMMITTED_COMMIT_ID.to_string(),
        short_hash: "uncommit".to_string(),
        author: String::new(),
        full_author: "uncommitted changes".to_string(),
        display_time: String::new(),
        message: "staged / unstaged / untracked changes".to_string(),
        files: dirty_files,
        files_loaded: true,
    }))
}

fn build_commit_review(commit: gix::Commit<'_>) -> Result<GitCommitReview> {
    let commit_id = commit.id.to_string();
    let short_hash = commit.short_id()?.to_string();
    let full_author = commit.author()?.name.to_string();
    let author = format_commit_author(&full_author);
    let display_time = format_commit_time(commit.time()?.seconds);
    let message = commit.message_raw_sloppy().to_string();

    Ok(GitCommitReview {
        commit: commit_id,
        short_hash,
        author,
        full_author,
        display_time,
        message,
        files: Vec::new(),
        files_loaded: false,
    })
}

pub fn load_git_commit_files(
    repo_root: &Path,
    repo: Option<&str>,
    commit_id: &str,
) -> Result<Vec<GitFileReview>> {
    let git_repo = gix::open(repo_root)
        .with_context(|| format!("failed to open git repository {}", repo_root.display()))?;
    let object = git_repo
        .rev_parse_single(commit_id)
        .with_context(|| format!("failed to resolve commit {commit_id}"))?
        .object()?;
    let commit = object
        .try_into_commit()
        .map_err(|error| anyhow::anyhow!("expected commit {commit_id}: {error}"))?;
    commit_files(&git_repo, &commit, repo, commit_id)
}

fn commit_files(
    repo_handle: &gix::Repository,
    commit: &gix::Commit<'_>,
    repo: Option<&str>,
    commit_id: &str,
) -> Result<Vec<GitFileReview>> {
    let new_tree = commit.tree()?;
    let old_tree = commit
        .parent_ids()
        .next()
        .and_then(|parent| parent.object().ok())
        .and_then(|object| object.try_into_commit().ok())
        .and_then(|parent| parent.tree().ok());
    let changes = repo_handle.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;
    let mut files = Vec::new();
    for change in changes {
        for path in diff_change_paths(&change, old_tree.as_ref(), &new_tree)? {
            if path.is_empty() {
                continue;
            }
            let project_relative_path = match repo {
                Some(repo_name) => format!("{repo_name}/{path}"),
                None => path.clone(),
            };
            files.push(GitFileReview {
                display_path: path,
                entry: FileEntry {
                    path: project_relative_path,
                    diff_source: DiffSource::CommitBacked {
                        commit: commit_id.to_string(),
                        repo: repo.map(ToString::to_string),
                    },
                },
            });
        }
    }
    files.sort_by(|left, right| left.display_path.cmp(&right.display_path));
    files.dedup_by(|left, right| left.display_path == right.display_path);
    Ok(files)
}

fn diff_change_paths(
    change: &gix::object::tree::diff::ChangeDetached,
    old_tree: Option<&gix::Tree<'_>>,
    new_tree: &gix::Tree<'_>,
) -> Result<Vec<String>> {
    let (path, tree) = match change {
        gix::object::tree::diff::ChangeDetached::Addition {
            location,
            entry_mode,
            ..
        } => {
            let tree = (entry_mode.kind() == gix::objs::tree::EntryKind::Tree).then_some(new_tree);
            (location.to_string(), tree)
        }
        gix::object::tree::diff::ChangeDetached::Modification {
            location,
            entry_mode,
            ..
        } => {
            if entry_mode.kind() == gix::objs::tree::EntryKind::Tree {
                return Ok(Vec::new());
            }
            (location.to_string(), None)
        }
        gix::object::tree::diff::ChangeDetached::Rewrite {
            location,
            entry_mode,
            ..
        } => {
            if entry_mode.kind() == gix::objs::tree::EntryKind::Tree {
                return Ok(Vec::new());
            }
            (location.to_string(), None)
        }
        gix::object::tree::diff::ChangeDetached::Deletion {
            location,
            entry_mode,
            ..
        } => {
            let tree = (entry_mode.kind() == gix::objs::tree::EntryKind::Tree)
                .then_some(old_tree)
                .flatten();
            (location.to_string(), tree)
        }
    };
    let Some(tree) = tree else {
        return Ok(vec![path]);
    };
    expand_tree_path(tree, &path)
}

fn expand_tree_path(tree: &gix::Tree<'_>, path: &str) -> Result<Vec<String>> {
    let Some(entry) = tree.lookup_entry_by_path(path)? else {
        return Ok(vec![path.to_string()]);
    };
    if entry.mode().kind() != gix::objs::tree::EntryKind::Tree {
        return Ok(vec![path.to_string()]);
    }

    let subtree = entry.object()?.try_into_tree().map_err(|error| {
        anyhow::anyhow!("expected tree while expanding changed path {path}: {error}")
    })?;
    let mut files = Vec::new();
    for entry in subtree.traverse().breadthfirst.files()? {
        if entry.mode.kind() == gix::objs::tree::EntryKind::Tree {
            continue;
        }
        let child = entry.filepath.to_string();
        files.push(if path.is_empty() {
            child
        } else {
            format!("{path}/{child}")
        });
    }
    Ok(files)
}

fn format_commit_time(seconds: i64) -> String {
    let Ok(seconds) = u64::try_from(seconds) else {
        return String::new();
    };
    let time = UNIX_EPOCH + Duration::from_secs(seconds);
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => String::new(),
    }
}

fn format_commit_author(value: &str) -> String {
    let name = value
        .split_whitespace()
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(value);

    let segments = name
        .split(['.', '-', '_'])
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if segments.len() > 1 {
        let compact = segments
            .iter()
            .enumerate()
            .filter_map(|(index, part)| format_author_segment(part, index == 0))
            .collect::<String>();
        return if compact.is_empty() {
            name.to_string()
        } else {
            compact
        };
    }

    compact_pinyinish_name(name).unwrap_or_else(|| name.to_string())
}

fn format_author_segment(value: &str, first: bool) -> Option<String> {
    let mut ch = value.chars().next()?;
    if !first {
        ch = ch.to_ascii_uppercase();
    }
    Some(ch.to_string())
}

fn compact_pinyinish_name(value: &str) -> Option<String> {
    let split = pinyinish_nasal_split(value)?;
    let (head, tail) = value.split_at(split);
    let first = head.chars().next()?.to_string();
    if tail.is_empty() {
        return Some(first);
    }
    if is_valid_pinyinish_nasal_syllable(tail) {
        let tail_first = tail.chars().next()?.to_ascii_uppercase();
        return Some(format!("{first}{tail_first}"));
    }
    let mut tail_chars = tail.chars();
    let tail_first = tail_chars.next()?.to_ascii_uppercase();
    Some(format!(
        "{first}{tail_first}{}",
        tail_chars.collect::<String>()
    ))
}

fn pinyinish_nasal_split(value: &str) -> Option<usize> {
    for marker in ["ang", "eng", "ing", "an", "en", "in"] {
        if let Some(index) = value.find(marker) {
            let split = index + marker.len();
            if split < value.len() && is_valid_pinyinish_nasal_syllable(&value[..split]) {
                return Some(split);
            }
        }
    }
    None
}

fn is_valid_pinyinish_nasal_syllable(value: &str) -> bool {
    let (initial, final_part) = split_pinyin_initial(value);
    match final_part {
        "ang" => matches!(
            initial,
            "b" | "p"
                | "m"
                | "f"
                | "d"
                | "t"
                | "n"
                | "l"
                | "g"
                | "k"
                | "h"
                | "zh"
                | "ch"
                | "sh"
                | "r"
                | "z"
                | "c"
                | "s"
                | "y"
                | "w"
        ),
        "eng" => matches!(
            initial,
            "b" | "p"
                | "m"
                | "f"
                | "d"
                | "t"
                | "n"
                | "l"
                | "g"
                | "k"
                | "h"
                | "zh"
                | "ch"
                | "sh"
                | "r"
                | "z"
                | "c"
                | "s"
                | "w"
        ),
        "ing" => matches!(
            initial,
            "b" | "p" | "m" | "d" | "t" | "n" | "l" | "j" | "q" | "x" | "y"
        ),
        "iang" => matches!(initial, "j" | "q" | "x" | "n" | "l"),
        "in" => matches!(initial, "b" | "p" | "m" | "n" | "l" | "j" | "q" | "x" | "y"),
        "an" | "en" | "ian" => !initial.is_empty(),
        _ => false,
    }
}

fn split_pinyin_initial(value: &str) -> (&str, &str) {
    for initial in ["zh", "ch", "sh"] {
        if let Some(rest) = value.strip_prefix(initial) {
            return (initial, rest);
        }
    }
    let Some(first) = value.get(..1) else {
        return ("", value);
    };
    if matches!(
        first,
        "b" | "p"
            | "m"
            | "f"
            | "d"
            | "t"
            | "n"
            | "l"
            | "g"
            | "k"
            | "h"
            | "j"
            | "q"
            | "x"
            | "r"
            | "z"
            | "c"
            | "s"
            | "y"
            | "w"
    ) {
        (first, &value[1..])
    } else {
        ("", value)
    }
}

fn normalize_slashes(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
#[path = "git_test.rs"]
mod git_test;
