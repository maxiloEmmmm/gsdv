use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use crate::{BranchCheckout, BranchInfo};

pub(crate) fn ensure_clean_repo(repo_root: &Path) -> Result<()> {
    let repo = open_repo(repo_root)?;
    let mut status = repo
        .status(gix::progress::Discard)?
        .untracked_files(gix::status::UntrackedFiles::Files)
        .into_iter(Vec::new())
        .with_context(|| format!("failed to inspect git status for {}", repo_root.display()))?;

    if status.next().is_some() {
        bail!("repo has staged, unstaged, or untracked changes");
    }

    Ok(())
}

pub(crate) fn load_branch_choices(repo_root: &Path) -> Result<(String, Vec<BranchInfo>)> {
    let repo = open_repo(repo_root)?;
    let current = current_branch_name(&repo).unwrap_or_else(|| "(detached)".to_string());

    let mut local_names = std::collections::BTreeSet::new();
    let mut branches = Vec::new();
    for name in branch_names(&repo, BranchKind::Local)? {
        local_names.insert(name.clone());
        branches.push(BranchInfo {
            label: name.clone(),
            checkout: BranchCheckout::Local(name),
        });
    }

    for remote in branch_names(&repo, BranchKind::Remote)? {
        if remote.ends_with("/HEAD") {
            continue;
        }
        let Some((_, local)) = remote.split_once('/') else {
            continue;
        };
        let local = local.to_string();
        if local_names.contains(&local) {
            continue;
        }
        branches.push(BranchInfo {
            label: remote.clone(),
            checkout: BranchCheckout::Remote { remote, local },
        });
    }

    branches.sort_by(|left, right| left.label.cmp(&right.label));
    Ok((current, branches))
}

pub(crate) fn checkout_branch(repo_root: &Path, checkout: &BranchCheckout) -> Result<()> {
    match checkout {
        BranchCheckout::Local(branch) => checkout_local_branch(repo_root, branch),
        BranchCheckout::Remote { remote, local } => {
            if local_branch_exists(repo_root, local)? {
                checkout_local_branch(repo_root, local)
            } else {
                create_local_branch_from_remote(repo_root, remote, local)?;
                checkout_local_branch(repo_root, local)
            }
        }
    }
}

fn open_repo(repo_root: &Path) -> Result<gix::Repository> {
    gix::open(repo_root)
        .with_context(|| format!("failed to open git repository {}", repo_root.display()))
}

fn current_branch_name(repo: &gix::Repository) -> Option<String> {
    let name = repo.head_name().ok()??;
    let text = name.shorten().to_string();
    if text.is_empty() { None } else { Some(text) }
}

enum BranchKind {
    Local,
    Remote,
}

fn branch_names(repo: &gix::Repository, kind: BranchKind) -> Result<Vec<String>> {
    let refs = repo.references()?;
    let iter = match kind {
        BranchKind::Local => refs.local_branches()?,
        BranchKind::Remote => refs.remote_branches()?,
    };
    let mut names = Vec::new();
    for reference in iter {
        let reference = reference.map_err(|error| anyhow::anyhow!("{error}"))?;
        names.push(reference.name().shorten().to_string());
    }
    names.sort();
    names.dedup();
    Ok(names)
}

fn local_branch_exists(repo_root: &Path, branch: &str) -> Result<bool> {
    let repo = open_repo(repo_root)?;
    Ok(repo
        .try_find_reference(format!("refs/heads/{branch}").as_str())?
        .is_some())
}

fn checkout_local_branch(repo_root: &Path, branch: &str) -> Result<()> {
    let repo = open_repo(repo_root)?;
    let reference = repo
        .find_reference(format!("refs/heads/{branch}").as_str())
        .with_context(|| format!("failed to find local branch {branch}"))?;
    let target = reference
        .target()
        .try_id()
        .context("branch does not point at an object id")?
        .to_owned();

    repo.edit_reference(gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: gix::refs::transaction::LogChange {
                mode: gix::refs::transaction::RefLog::AndReference,
                force_create_reflog: false,
                message: "checkout: moving branch".into(),
            },
            expected: gix::refs::transaction::PreviousValue::Any,
            new: gix::refs::Target::Symbolic(format!("refs/heads/{branch}").try_into()?),
        },
        name: "HEAD".try_into()?,
        deref: false,
    })?;

    checkout_tree(repo_root, target.into())?;
    Ok(())
}

fn create_local_branch_from_remote(repo_root: &Path, remote: &str, local: &str) -> Result<()> {
    let repo = open_repo(repo_root)?;
    let remote_ref = repo
        .find_reference(format!("refs/remotes/{remote}").as_str())
        .with_context(|| format!("failed to find remote branch {remote}"))?;
    let target = remote_ref
        .target()
        .try_id()
        .context("remote branch does not point at an object id")?
        .to_owned();
    repo.reference(
        format!("refs/heads/{local}"),
        target,
        gix::refs::transaction::PreviousValue::MustNotExist,
        format!("branch: Created from {remote}"),
    )?;
    Ok(())
}

fn checkout_tree(repo_root: &Path, target: gix::ObjectId) -> Result<()> {
    let repo = open_repo(repo_root)?;
    let old_files = repo
        .head_commit()
        .ok()
        .and_then(|commit| commit.tree().ok())
        .map(|tree| tree_files(&tree))
        .transpose()?
        .unwrap_or_default();

    let target_commit = repo.find_commit(target)?;
    let tree = target_commit.tree()?;
    let new_files = tree_files(&tree)?;
    let mut index = repo.index_from_tree(tree.id.as_ref())?;
    let mut options =
        repo.checkout_options(gix::worktree::stack::state::attributes::Source::IdMapping)?;
    options.overwrite_existing = true;

    let should_interrupt = AtomicBool::new(false);
    let files = gix::progress::Discard;
    let bytes = gix::progress::Discard;
    let outcome = gix::worktree::state::checkout(
        &mut index,
        repo_root,
        repo.objects.clone().into_arc()?,
        &files,
        &bytes,
        &should_interrupt,
        options,
    )?;
    if !outcome.collisions.is_empty() || !outcome.errors.is_empty() {
        bail!("checkout encountered filesystem conflicts");
    }
    index.write(Default::default())?;

    for removed in old_files.difference(&new_files) {
        let path = repo_root.join(removed);
        if path.is_file() || path.is_symlink() {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

fn tree_files(tree: &gix::Tree<'_>) -> Result<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    for entry in tree.traverse().breadthfirst.files()? {
        if entry.mode.kind() == gix::objs::tree::EntryKind::Tree {
            continue;
        }
        files.insert(entry.filepath.to_string());
    }
    Ok(files)
}

#[cfg(test)]
#[path = "git_backend_test.rs"]
mod git_backend_test;
