use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceStatus {
    Ok,
    Bad,
}

impl Default for ProvenanceStatus {
    fn default() -> Self {
        Self::Ok
    }
}

impl ProvenanceStatus {
    pub fn display(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Bad => "bad",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitProvenance {
    Commits(Vec<String>),
    Missing,
    Bad,
}

impl CommitProvenance {
    pub fn display(&self) -> &'static str {
        match self {
            Self::Commits(_) => "ok",
            Self::Missing => "无",
            Self::Bad => "bad",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    CommitBacked {
        commit: String,
        repo: Option<String>,
    },
    WorkingTree {
        repo: Option<String>,
        staged: bool,
        unstaged: bool,
        untracked: bool,
    },
    NonCommitFallback {
        hint: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub diff_source: DiffSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoBucket {
    pub repo: Option<String>,
    pub unmatched: bool,
    pub files: Vec<FileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeGroup {
    pub plan_id: String,
    pub plan_title: String,
    pub task_index: usize,
    pub task_name: String,
    pub provenance_status: ProvenanceStatus,
    pub summary_status: ProvenanceStatus,
    pub commit_provenance: CommitProvenance,
    pub file_hints: Vec<String>,
    pub repo_buckets: Vec<RepoBucket>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanProvenance {
    pub id: String,
    pub title: String,
    pub change_groups: Vec<ChangeGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseProvenance {
    pub phase_number: String,
    pub phase_dir: PathBuf,
    pub plans: Vec<PlanProvenance>,
}

#[derive(Debug, Clone, Default)]
pub struct LoadPhaseOptions<'a> {
    pub workstream: Option<&'a str>,
}

#[derive(Debug, Deserialize, Default)]
struct PlanningConfig {
    #[serde(default)]
    sub_repos: Vec<String>,
}

#[derive(Debug, Clone)]
struct ParsedPlan {
    id: String,
    title: String,
    tasks: Vec<ParsedTask>,
}

#[derive(Debug, Clone)]
struct ParsedTask {
    name: String,
    file_hints: Vec<String>,
    status: ProvenanceStatus,
}

#[derive(Debug, Default)]
struct SummaryCommitSection {
    entries: BTreeMap<usize, CommitProvenance>,
    status: ProvenanceStatus,
}

pub fn load_phase_provenance(
    project_root: &Path,
    phase_number: &str,
    options: LoadPhaseOptions<'_>,
) -> Result<PhaseProvenance> {
    let phase_dir = resolve_phase_directory(project_root, phase_number, options.workstream)?;
    let config = load_config(project_root)?;
    let mut plans = Vec::new();

    for plan_path in plan_paths(&phase_dir)? {
        let plan_content = fs::read_to_string(&plan_path)
            .with_context(|| format!("failed to read {}", plan_path.display()))?;
        let parsed_plan = parse_plan(
            &plan_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| anyhow!("invalid plan file name"))?,
            &plan_content,
        );
        let summary_path = plan_path.with_file_name(
            plan_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .replace("-PLAN.md", "-SUMMARY.md"),
        );
        let summary = if summary_path.exists() {
            let content = fs::read_to_string(&summary_path)
                .with_context(|| format!("failed to read {}", summary_path.display()))?;
            parse_task_commits(&content)
        } else {
            SummaryCommitSection::default()
        };

        let mut change_groups = Vec::new();
        for (index, task) in parsed_plan.tasks.iter().enumerate() {
            let commit_provenance = summary
                .entries
                .get(&(index + 1))
                .cloned()
                .unwrap_or(CommitProvenance::Missing);
            let repo_buckets = build_repo_buckets(
                project_root,
                &config.sub_repos,
                &commit_provenance,
                &task.file_hints,
            )?;

            change_groups.push(ChangeGroup {
                plan_id: parsed_plan.id.clone(),
                plan_title: parsed_plan.title.clone(),
                task_index: index + 1,
                task_name: task.name.clone(),
                provenance_status: task.status,
                summary_status: summary.status,
                commit_provenance,
                file_hints: task.file_hints.clone(),
                repo_buckets,
            });
        }

        plans.push(PlanProvenance {
            id: parsed_plan.id,
            title: parsed_plan.title,
            change_groups,
        });
    }

    Ok(PhaseProvenance {
        phase_number: normalize_phase_number(phase_number),
        phase_dir,
        plans,
    })
}

fn load_config(project_root: &Path) -> Result<PlanningConfig> {
    let config_path = project_root.join(".planning/config.json");
    if !config_path.exists() {
        return Ok(PlanningConfig::default());
    }
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", config_path.display()))
}

fn resolve_phase_directory(
    project_root: &Path,
    phase_number: &str,
    workstream: Option<&str>,
) -> Result<PathBuf> {
    let phase_root = match workstream {
        Some(name) => project_root
            .join(".planning/workstreams")
            .join(name)
            .join("phases"),
        None => project_root.join(".planning/phases"),
    };
    let target = normalize_phase_number(phase_number);
    let entries = fs::read_dir(&phase_root)
        .with_context(|| format!("failed to read {}", phase_root.display()))?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let prefix = name.split('-').next().unwrap_or_default();
        if normalize_phase_number(prefix) == target {
            return Ok(entry.path());
        }
    }
    bail!(
        "phase {} not found under {}",
        phase_number,
        phase_root.display()
    )
}

fn plan_paths(phase_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(phase_dir)
        .with_context(|| format!("failed to read {}", phase_dir.display()))?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if name.ends_with("-PLAN.md") {
                Some(path)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| {
        let left_key = left
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let right_key = right
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        left_key.cmp(right_key)
    });
    Ok(paths)
}

fn parse_plan(file_name: &str, content: &str) -> ParsedPlan {
    let id = file_name.trim_end_matches("-PLAN.md").to_string();
    let title = extract_heading(content).unwrap_or_else(|| id.clone());
    let (frontmatter, body) = split_frontmatter(content);
    let plan_file_hints = extract_frontmatter_files(frontmatter);
    let task_blocks = extract_task_blocks(body);
    let mut tasks = Vec::new();

    if task_blocks.is_empty() {
        tasks.push(ParsedTask {
            name: "Malformed task block".to_string(),
            file_hints: plan_file_hints,
            status: ProvenanceStatus::Bad,
        });
    } else {
        for (index, block) in task_blocks.iter().enumerate() {
            let name = extract_xml_tag(block, "name")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("Task {}", index + 1));
            let file_hints = extract_xml_tag(block, "files")
                .map(parse_csvish_list)
                .filter(|items| !items.is_empty())
                .unwrap_or_else(|| plan_file_hints.clone());
            let status = if block.contains("</task>") && extract_xml_tag(block, "name").is_some() {
                ProvenanceStatus::Ok
            } else {
                ProvenanceStatus::Bad
            };
            tasks.push(ParsedTask {
                name,
                file_hints,
                status,
            });
        }
    }

    ParsedPlan { id, title, tasks }
}

fn split_frontmatter(content: &str) -> (&str, &str) {
    if let Some(rest) = content.strip_prefix("---\n") {
        if let Some(idx) = rest.find("\n---\n") {
            let (frontmatter, tail) = rest.split_at(idx);
            return (frontmatter, &tail["\n---\n".len()..]);
        }
    }
    ("", content)
}

fn extract_heading(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        line.strip_prefix('#')
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
    })
}

fn extract_frontmatter_files(frontmatter: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut capture = false;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed == "files_modified:" || trimmed == "files-modified:" {
            capture = true;
            continue;
        }
        if capture {
            if let Some(item) = trimmed.strip_prefix("- ") {
                files.push(item.trim().to_string());
                continue;
            }
            if trimmed.is_empty() {
                continue;
            }
            if !line.starts_with(' ') && !line.starts_with('\t') {
                break;
            }
        }
    }
    files
}

fn extract_task_blocks(content: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut search = content;
    while let Some(start) = search.find("<task") {
        let after_start = &search[start..];
        if let Some(end) = after_start.find("</task>") {
            let block = &after_start[..end + "</task>".len()];
            blocks.push(block.to_string());
            search = &after_start[end + "</task>".len()..];
        } else {
            blocks.push(after_start.to_string());
            break;
        }
    }
    blocks
}

fn extract_xml_tag<'a>(block: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = block.find(&open)?;
    let rest = &block[start + open.len()..];
    let end = rest.find(&close)?;
    Some(&rest[..end])
}

fn parse_csvish_list(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn parse_task_commits(content: &str) -> SummaryCommitSection {
    let mut result = SummaryCommitSection {
        status: ProvenanceStatus::Ok,
        ..SummaryCommitSection::default()
    };
    let Some(section) = extract_markdown_section(content, "## Task Commits") else {
        return result;
    };

    for line in section
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some((task_index, commit_provenance)) = parse_task_commit_line(line) else {
            result.status = ProvenanceStatus::Bad;
            continue;
        };
        result.entries.insert(task_index, commit_provenance);
    }

    result
}

fn extract_markdown_section<'a>(content: &'a str, heading: &str) -> Option<&'a str> {
    let start = content.find(heading)?;
    let rest = &content[start + heading.len()..];
    let end = rest.find("\n## ").unwrap_or(rest.len());
    Some(rest[..end].trim())
}

fn parse_task_commit_line(line: &str) -> Option<(usize, CommitProvenance)> {
    let task_marker = line.find("Task ")?;
    let rest = &line[task_marker + "Task ".len()..];
    let colon = rest.find(':')?;
    let task_number = rest[..colon].trim().parse().ok()?;
    let hashes = extract_backtick_tokens(line);
    if hashes.is_empty() {
        return Some((task_number, CommitProvenance::Bad));
    }
    if hashes.iter().all(|hash| is_commit_hash(hash)) {
        return Some((task_number, CommitProvenance::Commits(hashes)));
    }
    Some((task_number, CommitProvenance::Bad))
}

fn extract_backtick_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('`') {
        let tail = &rest[start + 1..];
        if let Some(end) = tail.find('`') {
            tokens.push(tail[..end].trim().to_string());
            rest = &tail[end + 1..];
        } else {
            break;
        }
    }
    tokens
}

fn is_commit_hash(value: &str) -> bool {
    let len = value.len();
    (7..=40).contains(&len) && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn build_repo_buckets(
    project_root: &Path,
    sub_repos: &[String],
    commit_provenance: &CommitProvenance,
    file_hints: &[String],
) -> Result<Vec<RepoBucket>> {
    let mut buckets: BTreeMap<Option<String>, Vec<FileEntry>> = BTreeMap::new();

    match commit_provenance {
        CommitProvenance::Commits(commits) => {
            for commit in commits {
                for file in git_changed_files(project_root, commit)? {
                    let repo = sub_repos
                        .iter()
                        .find(|repo| file.starts_with(&format!("{repo}/")))
                        .cloned();
                    buckets.entry(repo.clone()).or_default().push(FileEntry {
                        path: file,
                        diff_source: DiffSource::CommitBacked {
                            commit: commit.clone(),
                            repo,
                        },
                    });
                }
            }
        }
        CommitProvenance::Missing | CommitProvenance::Bad => {
            for hint in file_hints {
                let repo = sub_repos
                    .iter()
                    .find(|repo| hint.starts_with(&format!("{repo}/")))
                    .cloned();
                buckets.entry(repo.clone()).or_default().push(FileEntry {
                    path: hint.clone(),
                    diff_source: DiffSource::NonCommitFallback {
                        hint: "plan/task file hint".to_string(),
                    },
                });
            }
        }
    }

    let mut repo_buckets = buckets
        .into_iter()
        .map(|(repo, mut files)| {
            dedupe_files(&mut files);
            RepoBucket {
                unmatched: repo.is_none(),
                repo,
                files,
            }
        })
        .collect::<Vec<_>>();
    repo_buckets.sort_by(|left, right| left.repo.cmp(&right.repo));
    Ok(repo_buckets)
}

fn dedupe_files(files: &mut Vec<FileEntry>) {
    let mut seen = BTreeSet::new();
    files.retain(|entry| {
        let key = (
            entry.path.clone(),
            match &entry.diff_source {
                DiffSource::CommitBacked { commit, repo } => {
                    format!("commit:{commit}:{}", repo.clone().unwrap_or_default())
                }
                DiffSource::WorkingTree {
                    repo,
                    staged,
                    unstaged,
                    untracked,
                } => format!(
                    "working:{}:{staged}:{unstaged}:{untracked}",
                    repo.clone().unwrap_or_default()
                ),
                DiffSource::NonCommitFallback { hint } => format!("fallback:{hint}"),
            },
        );
        seen.insert(key)
    });
}

fn git_changed_files(project_root: &Path, commit: &str) -> Result<Vec<String>> {
    let repo = gix::open(project_root)
        .with_context(|| format!("failed to open git repository {}", project_root.display()))?;
    let id = match repo.rev_parse_single(commit) {
        Ok(id) => id,
        Err(_) => return Ok(Vec::new()),
    };
    let object = match id.object() {
        Ok(object) => object,
        Err(_) => return Ok(Vec::new()),
    };
    let commit = match object.try_into_commit() {
        Ok(commit) => commit,
        Err(_) => return Ok(Vec::new()),
    };
    let new_tree = commit.tree()?;
    let old_tree = commit
        .parent_ids()
        .next()
        .and_then(|parent| parent.object().ok())
        .and_then(|object| object.try_into_commit().ok())
        .and_then(|parent| parent.tree().ok());
    let mut files = Vec::new();
    for change in repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)? {
        files.extend(gix_diff_change_paths(
            &change,
            old_tree.as_ref(),
            &new_tree,
        )?);
    }
    files.retain(|line| !line.is_empty());
    files.sort();
    files.dedup();
    Ok(files)
}

fn gix_diff_change_paths(
    change: &gix::object::tree::diff::ChangeDetached,
    old_tree: Option<&gix::Tree<'_>>,
    new_tree: &gix::Tree<'_>,
) -> Result<Vec<String>> {
    let (path, tree) = match change {
        gix::object::tree::diff::ChangeDetached::Addition { location, .. }
        | gix::object::tree::diff::ChangeDetached::Modification { location, .. }
        | gix::object::tree::diff::ChangeDetached::Rewrite { location, .. } => {
            (location.to_string(), Some(new_tree))
        }
        gix::object::tree::diff::ChangeDetached::Deletion { location, .. } => {
            (location.to_string(), old_tree)
        }
    };
    let Some(tree) = tree else {
        return Ok(vec![path]);
    };
    expand_gix_tree_path(tree, &path)
}

fn expand_gix_tree_path(tree: &gix::Tree<'_>, path: &str) -> Result<Vec<String>> {
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

fn normalize_phase_number(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(stripped) = trimmed.strip_prefix('0') {
        if stripped.is_empty() {
            "0".to_string()
        } else {
            stripped.to_string()
        }
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
#[path = "provenance_test.rs"]
mod provenance_test;
