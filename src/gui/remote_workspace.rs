//! SSH Remote Workspace 的持久化模型、远端文件访问和命令构造。
//!
//! 本模块不持有 egui 状态。阻塞进程和文件操作只能由后台任务调用。

use crate::gui::agent::AgentKind;
use crate::gui::data::{
    AgentColumnViewData, AgentFocusViewData, AgentRowViewData, NetworkSettings, SubagentViewData,
};
use crate::gui::workflow::{
    self, WorkflowMutationRequest, WorkflowSaveRequest, WorkflowSaveSuccess, WorkflowTree,
};
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// SSH 密码认证配置。
///
/// 适用场景：远端账号只开放密码登录。例：`123123` -> askpass 自动提交。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PasswordAuth {
    /// 明文 SSH 密码；产品要求直接写入本地 store。
    pub password: String,
}

/// SSH 私钥认证配置。
///
/// 适用场景：公钥已部署到远端。例：`~/.ssh/id_ed25519` -> `ssh -i`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PrivateKeyAuth {
    /// 本机私钥文件路径；不支持私钥 passphrase 自动输入。
    pub private_key: PathBuf,
}

/// Remote Workspace 使用的互斥认证方式。
///
/// 适用场景：创建连接时只能选择密码或私钥。例：Password -> 不追加 `-i`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RemoteAuth {
    /// 明文密码认证。
    Password(PasswordAuth),
    /// 指定私钥认证。
    PrivateKey(PrivateKeyAuth),
}

/// 本机持久化的 Remote Workspace 连接配置。
///
/// 适用场景：Workspace Rail 中的一个远端入口。例：`dev -> shibo@host:22`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RemoteWorkspaceConfig {
    /// Remote Workspace 永久存储 key；名称和连接参数变化时保持不变。
    #[serde(default = "new_workspace_key")]
    pub workspace_key: String,
    /// Workspace Rail 展示的自定义名称。
    pub name: String,
    /// SSH 主机名或 IP 地址。
    pub host: String,
    /// SSH 端口，默认 22。
    pub port: u16,
    /// SSH 登录用户名。
    pub username: String,
    /// 二选一的 SSH 认证配置。
    pub auth: RemoteAuth,
}

/// 已探测出的远端命令 shell。
///
/// 适用场景：为 agent 构造远端 `cd + exec`。例：Windows PowerShell -> Set-Location。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteShell {
    /// Linux/macOS POSIX shell。
    Posix,
    /// Windows PowerShell。
    PowerShell,
    /// Windows cmd.exe。
    Cmd,
}

/// 远端单个 workspace 的 Agent 元数据快照。
///
/// 适用场景：Outline 点击项目后构建现有 Agent 网格。例：store workspace + sidecar -> UI。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RemoteProjectSnapshot {
    /// 远端 store 中的原始数组下标，用于回写 active。
    pub store_index: usize,
    /// 远端 workspace 绝对路径。
    pub path: PathBuf,
    /// 主 agent 类型。
    pub agent_kind: AgentKind,
    /// 主 agent 模型覆盖。
    pub agent_model: Option<String>,
    /// Codex 模型供应商覆盖。
    pub agent_model_provider: Option<String>,
    /// Agent effort 覆盖。
    pub agent_effort: Option<String>,
    /// Codex fast mode 覆盖。
    pub agent_fast_mode: Option<bool>,
    /// Agent 工作目录覆盖。
    pub agent_work_dir: Option<PathBuf>,
    /// 主 agent 稳定 ID。
    pub agent_id: String,
    /// 主 agent resume session ID。
    pub session_id: Option<String>,
    /// Workspace 下的 subagents。
    pub subagents: Vec<SubagentViewData>,
    /// 可见 Agent 行布局。
    pub rows: Vec<AgentRowViewData>,
    /// 当前 Agent 网格焦点。
    pub focus: Option<AgentFocusViewData>,
}

/// SSH 加载返回的完整 Remote Workspace 快照。
///
/// 适用场景：首次连接和切回 Rail 项时刷新。例：远端 active=1 -> selected_store_index=1。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RemoteWorkspaceSnapshot {
    /// 远端 gsdv 为 Agent 子进程保存的代理配置。
    pub network_settings: NetworkSettings,
    /// 远端 store 中当前激活 workspace 的原始下标。
    pub selected_store_index: Option<usize>,
    /// 已过滤不存在目录的远端 workspace。
    pub projects: Vec<RemoteProjectSnapshot>,
}

/// 回写远端 Agent sidecar 的布局载荷。
///
/// 适用场景：本机调整远端布局。例：移动一列 -> 保留远端最新 session 后覆盖 rows。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RemoteLayoutPayload {
    /// 目标远端 workspace 路径。
    pub workspace_path: PathBuf,
    /// 主 agent 类型。
    pub agent_kind: AgentKind,
    /// 主 agent 模型覆盖。
    pub agent_model: Option<String>,
    /// Codex 模型供应商覆盖。
    pub agent_model_provider: Option<String>,
    /// 主 agent effort 覆盖。
    pub agent_effort: Option<String>,
    /// 主 agent fast mode 覆盖。
    pub agent_fast_mode: Option<bool>,
    /// 主 agent 工作目录覆盖。
    pub agent_work_dir: Option<PathBuf>,
    /// 最新 subagent 配置。
    pub subagents: Vec<SubagentViewData>,
    /// 最新 Agent 行布局。
    pub rows: Vec<AgentRowViewData>,
    /// 最新 Agent 焦点。
    pub focus: Option<AgentFocusViewData>,
}

/// 返回 Remote Workspace 默认 SSH 端口。
///
/// 适用场景：创建表单初始化。例：空表单 -> 22。
pub const fn default_ssh_port() -> u16 {
    22
}

/// 生成 Remote Workspace 永久存储 key。
///
/// 适用场景：创建新的 Remote Rail 项。例：随机值 -> `remote-<16位十六进制>`。
pub fn new_workspace_key() -> String {
    format!("remote-{:016x}", rand::random::<u64>())
}

/// 将配置转换成 OpenSSH 目标参数。
///
/// 适用场景：所有 SSH 子命令共享。例：shibo + host -> `shibo@host`。
pub fn ssh_target(config: &RemoteWorkspaceConfig) -> String {
    format!("{}@{}", config.username, config.host)
}

/// 判断两份 Remote 配置是否只可能在显示名称上不同。
///
/// 适用场景：编辑名称无需重连。例：同 host/user/auth、不同 name -> true。
pub fn same_connection(left: &RemoteWorkspaceConfig, right: &RemoteWorkspaceConfig) -> bool {
    left.workspace_key == right.workspace_key
        && left.host == right.host
        && left.port == right.port
        && left.username == right.username
        && left.auth == right.auth
}

/// 返回每条独立 OpenSSH 连接共用的参数。
///
/// 适用场景：元数据命令和 Agent 各自直连。例：端口 2222 -> `-p 2222`。
pub fn ssh_connection_args(config: &RemoteWorkspaceConfig) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        config.port.to_string(),
        "-o".to_string(),
        "ConnectTimeout=5".to_string(),
        "-o".to_string(),
        "ServerAliveInterval=10".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=3".to_string(),
        "-o".to_string(),
        "StrictHostKeyChecking=accept-new".to_string(),
    ];
    match &config.auth {
        RemoteAuth::Password(_) => {
            args.extend([
                "-o".to_string(),
                "PreferredAuthentications=password".to_string(),
                "-o".to_string(),
                "NumberOfPasswordPrompts=1".to_string(),
            ]);
        }
        RemoteAuth::PrivateKey(auth) => {
            args.extend([
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                "-o".to_string(),
                "IdentitiesOnly=yes".to_string(),
                "-i".to_string(),
                auth.private_key.display().to_string(),
            ]);
        }
    }
    args
}

/// 向独立 OpenSSH 命令追加共用参数。
///
/// 适用场景：`Command` 元数据请求复用 Agent 相同认证参数。
pub fn append_connection_args(command: &mut Command, config: &RemoteWorkspaceConfig) {
    command.args(ssh_connection_args(config));
}

/// 为密码认证配置 OpenSSH askpass 环境。
///
/// 适用场景：主连接首次认证，避免密码进入命令参数。例：Password -> 当前 gsdv 子进程输出密码。
pub fn configure_askpass(
    command: &mut Command,
    config: &RemoteWorkspaceConfig,
) -> Result<Option<PathBuf>> {
    let RemoteAuth::Password(auth) = &config.auth else {
        return Ok(None);
    };
    let executable = std::env::current_exe().context("resolve gsdv executable for SSH askpass")?;
    let password_path = std::env::temp_dir().join(format!(
        "gsdv-askpass-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    fs::write(&password_path, auth.password.as_bytes()).context("write temporary SSH password")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&password_path)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&password_path, permissions)?;
    }
    command
        .env("SSH_ASKPASS", executable)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", "gsdv-ssh-askpass")
        .env("GSDV_SSH_ASKPASS", "1")
        .env("GSDV_SSH_PASSWORD_FILE", &password_path)
        .stdin(Stdio::null());
    Ok(Some(password_path))
}

/// 通过独立 SSH 连接执行远端命令并返回 stdout。
///
/// 适用场景：shell 探测和 helper 协议。例：退出码非零 -> 带 stderr 报错。
pub fn run_ssh_command(config: &RemoteWorkspaceConfig, remote_command: &str) -> Result<Vec<u8>> {
    let mut command = Command::new("ssh");
    append_connection_args(&mut command, config);
    let password_path = configure_askpass(&mut command, config)?;
    command.arg(ssh_target(config)).arg(remote_command);
    let output = command.output().context("run remote SSH command");
    if let Some(path) = password_path {
        let _ = fs::remove_file(path);
    }
    let output = output?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        bail!(ssh_error_message(&output.stderr))
    }
}

/// 自动识别远端默认 shell。
///
/// 适用场景：构造远端 Agent 命令。例：`%COMSPEC%` 被展开 -> cmd。
pub fn detect_remote_shell(config: &RemoteWorkspaceConfig) -> Result<RemoteShell> {
    if let Ok(posix) = run_ssh_command(config, "printf gsdv-posix")
        && String::from_utf8_lossy(&posix).trim() == "gsdv-posix"
    {
        return Ok(RemoteShell::Posix);
    }
    if let Ok(pshome) = run_ssh_command(config, "echo $PSHOME") {
        let pshome = String::from_utf8_lossy(&pshome);
        let pshome = pshome.trim();
        if !pshome.is_empty() && pshome != "$PSHOME" {
            return Ok(RemoteShell::PowerShell);
        }
    }
    if let Ok(comspec) = run_ssh_command(config, "echo %COMSPEC%") {
        let comspec = String::from_utf8_lossy(&comspec);
        if !comspec.trim().is_empty() && !comspec.contains("%COMSPEC%") {
            return Ok(RemoteShell::Cmd);
        }
    }
    bail!("unable to detect remote POSIX, PowerShell, or cmd shell")
}

/// 单个远端 workspace 的批量读取请求。
struct RemoteProjectReadRequest {
    /// 远端 store.workspaces 中的原始下标。
    store_index: usize,
    /// 需要检查是否存在的 workspace 根目录。
    path: PathBuf,
    /// HOME 相对 sidecar 路径。
    sidecar_path: String,
}

/// 单个远端 workspace 的批量读取结果。
struct RemoteProjectReadResult {
    /// workspace 根目录是否仍然存在。
    directory_exists: bool,
    /// sidecar JSON；文件不存在时为空对象。
    sidecar: Value,
}

/// 通过系统 SSH 直接读取远端 store 与布局 sidecar。
///
/// 适用场景：远端不安装任何新增组件。例：现有 ~/.gsdv -> RemoteWorkspaceSnapshot。
pub fn load_remote_snapshot(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
) -> Result<RemoteWorkspaceSnapshot> {
    let store = read_remote_json(config, shell, ".gsdv/store", false)?;
    let network_settings = store
        .get("network_settings")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .context("parse remote ~/.gsdv/store network_settings")?
        .unwrap_or_default();
    let selected_store_index = store
        .get("active")
        .and_then(Value::as_u64)
        .map(|value| value as usize);
    let workspaces = store
        .get("workspaces")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("remote ~/.gsdv/store workspaces is not an array"))?;
    let requests = workspaces
        .iter()
        .enumerate()
        .filter_map(|(store_index, workspace)| {
            let path = workspace.get("path")?.as_str().map(PathBuf::from)?;
            let sidecar_path = format!(
                ".gsdv/workspaces/{}/subagents.json",
                crate::gui::data::workspace_store_key(&path)
            );
            Some(RemoteProjectReadRequest {
                store_index,
                path,
                sidecar_path,
            })
        })
        .collect::<Vec<_>>();
    let batch = read_remote_projects_batch(config, shell, &requests)?;
    let mut projects = Vec::new();
    for request in requests {
        let Some(workspace) = workspaces.get(request.store_index) else {
            continue;
        };
        let Some(result) = batch.get(&request.store_index) else {
            bail!("remote batch omitted workspace {}", request.store_index);
        };
        if !result.directory_exists {
            continue;
        }
        projects.push(project_snapshot(
            request.store_index,
            request.path,
            workspace,
            &result.sidecar,
        )?);
    }
    Ok(RemoteWorkspaceSnapshot {
        network_settings,
        selected_store_index,
        projects,
    })
}

/// 通过系统 SSH 只更新远端 store.active。
///
/// 适用场景：Outline 切换远端项目。例：原始下标 3 -> 保留其余 store 字段。
pub fn save_remote_active(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    store_index: usize,
) -> Result<()> {
    let mut store = read_remote_json(config, shell, ".gsdv/store", false)?;
    store["active"] = Value::from(store_index);
    write_remote_json(config, shell, ".gsdv/store", &store)
}

/// 通过系统 SSH 合并并保存远端 Agent 布局。
///
/// 适用场景：本机布局变更。例：rows 更新 -> helper 保留最新 session_id。
pub fn save_remote_layout(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    payload: &RemoteLayoutPayload,
) -> Result<()> {
    let mut store = read_remote_json(config, shell, ".gsdv/store", false)?;
    let workspace_path = payload.workspace_path.to_string_lossy();
    let workspace = store
        .get_mut("workspaces")
        .and_then(Value::as_array_mut)
        .and_then(|workspaces| {
            workspaces.iter_mut().find(|workspace| {
                workspace.get("path").and_then(Value::as_str) == Some(workspace_path.as_ref())
            })
        })
        .ok_or_else(|| anyhow!("remote workspace is missing from store"))?;
    workspace["agent_kind"] = serde_json::to_value(payload.agent_kind)?;
    set_optional_json_string(workspace, "agent_model", payload.agent_model.as_deref());
    set_optional_json_string(
        workspace,
        "agent_model_provider",
        payload.agent_model_provider.as_deref(),
    );
    set_optional_json_string(workspace, "agent_effort", payload.agent_effort.as_deref());
    workspace["agent_fast_mode"] = payload
        .agent_fast_mode
        .map(Value::from)
        .unwrap_or(Value::Null);
    set_optional_json_string(
        workspace,
        "agent_work_dir",
        payload
            .agent_work_dir
            .as_ref()
            .map(|path| path.to_string_lossy())
            .as_deref(),
    );
    write_remote_json(config, shell, ".gsdv/store", &store)?;

    let sidecar_path = format!(
        ".gsdv/workspaces/{}/subagents.json",
        crate::gui::data::workspace_store_key(&payload.workspace_path)
    );
    let latest = read_remote_json(config, shell, &sidecar_path, true)?;
    let mut subagents = payload.subagents.clone();
    preserve_session_ids(&latest, &mut subagents);
    let sidecar = serde_json::json!({
        "subagents": subagents,
        "columns": Vec::<AgentColumnViewData>::new(),
        "rows": payload.rows,
        "focus": payload.focus,
    });
    write_remote_json(config, shell, &sidecar_path, &sidecar)
}

/// 通过 SSH 加载当前远端项目的 workflow tree。
///
/// 适用场景：Remote Workspace 的 Quick Modal 打开或刷新。
/// 例：`/repo/gsdv-spec -> WorkflowTree`。
pub(super) fn load_remote_workflow_tree(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    workspace_path: &Path,
) -> Result<WorkflowTree, String> {
    let command = remote_workflow_inventory_command(shell, workspace_path);
    let output = run_ssh_command(config, &command).map_err(|error| error.to_string())?;
    let documents = parse_remote_workflow_inventory(&output).map_err(|error| error.to_string())?;
    workflow::load_workflow_tree_from_documents(workspace_path, &documents)
}

/// 通过 SSH 保存当前远端项目的 task/step 文本。
///
/// 适用场景：Remote Quick Modal 执行 Cmd+S。
/// 例：`task desc changed -> 原子覆盖远端 task.md`。
pub(super) fn save_remote_workflow_step(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    workspace_path: &Path,
    request: WorkflowSaveRequest,
) -> Result<WorkflowSaveSuccess, String> {
    let absolute = remote_workflow_path(shell, workspace_path, &request.task_path)?;
    let content = read_remote_workflow_file(config, shell, &absolute)?;
    let (next_content, saved) = workflow::save_workflow_step_content(&content, &request)?;
    write_remote_workflow_file(config, shell, &absolute, next_content.as_bytes())?;
    Ok(saved)
}

/// 通过 SSH 执行完整的 workflow tree mutation。
///
/// 适用场景：Remote Quick Modal 的创建、重命名、合并和删除菜单。
/// 例：`RenameProject a -> b`。
pub(super) fn apply_remote_workflow_mutation(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    workspace_path: &Path,
    request: WorkflowMutationRequest,
) -> Result<(), String> {
    match &request {
        WorkflowMutationRequest::AddStep { task_path, .. }
        | WorkflowMutationRequest::RenameStep { task_path, .. }
        | WorkflowMutationRequest::DeleteStep { task_path, .. }
        | WorkflowMutationRequest::MergeSteps { task_path, .. } => {
            let absolute = remote_workflow_path(shell, workspace_path, task_path)?;
            let content = read_remote_workflow_file(config, shell, &absolute)?;
            let next_content = workflow::apply_workflow_task_content_mutation(&content, &request)?;
            write_remote_workflow_file(config, shell, &absolute, next_content.as_bytes())
        }
        WorkflowMutationRequest::InitRoot => {
            let relative = PathBuf::from("gsdv-spec/root.md");
            let absolute = remote_workflow_path(shell, workspace_path, &relative)?;
            run_remote_workflow_command(
                config,
                remote_workflow_create_file_command(shell, &absolute, None, true),
            )
        }
        WorkflowMutationRequest::AddProject { project_key } => {
            let project_key = workflow::validate_workflow_key(project_key)?;
            let relative = PathBuf::from("gsdv-spec")
                .join("ps")
                .join(project_key)
                .join("root.md");
            let absolute = remote_workflow_path(shell, workspace_path, &relative)?;
            run_remote_workflow_command(
                config,
                remote_workflow_create_file_command(shell, &absolute, None, false),
            )
        }
        WorkflowMutationRequest::AddTask {
            project_key,
            task_key,
        } => {
            let project_key = workflow::validate_workflow_key(project_key)?;
            let task_key = workflow::validate_workflow_key(task_key)?;
            let project_relative = PathBuf::from("gsdv-spec").join("ps").join(project_key);
            let relative = project_relative.join(format!("task-{task_key}.md"));
            let project = remote_workflow_path(shell, workspace_path, &project_relative)?;
            let absolute = remote_workflow_path(shell, workspace_path, &relative)?;
            run_remote_workflow_command(
                config,
                remote_workflow_create_file_command(shell, &absolute, Some(&project), false),
            )
        }
        WorkflowMutationRequest::RenameProject {
            project_key,
            new_key,
        } => {
            let project_key = workflow::validate_workflow_key(project_key)?;
            let new_key = workflow::validate_workflow_key(new_key)?;
            if project_key == new_key {
                return Ok(());
            }
            let projects = PathBuf::from("gsdv-spec").join("ps");
            let old_path =
                remote_workflow_path(shell, workspace_path, &projects.join(project_key))?;
            let new_path = remote_workflow_path(shell, workspace_path, &projects.join(new_key))?;
            run_remote_workflow_command(
                config,
                remote_workflow_rename_command(shell, &old_path, &new_path, true),
            )
        }
        WorkflowMutationRequest::RenameTask { task_path, new_key } => {
            let new_key = workflow::validate_workflow_key(new_key)?;
            let old_path = remote_workflow_path(shell, workspace_path, task_path)?;
            let parent = task_path
                .parent()
                .ok_or_else(|| format!("task parent not found: {}", task_path.display()))?;
            let target = parent.join(format!("task-{new_key}.md"));
            if target == *task_path {
                return Ok(());
            }
            let new_path = remote_workflow_path(shell, workspace_path, &target)?;
            run_remote_workflow_command(
                config,
                remote_workflow_rename_command(shell, &old_path, &new_path, false),
            )
        }
        WorkflowMutationRequest::DeleteProject { project_key } => {
            let project_key = workflow::validate_workflow_key(project_key)?;
            let relative = PathBuf::from("gsdv-spec").join("ps").join(project_key);
            let absolute = remote_workflow_path(shell, workspace_path, &relative)?;
            run_remote_workflow_command(
                config,
                remote_workflow_delete_command(shell, &absolute, true),
            )
        }
        WorkflowMutationRequest::DeleteTask { task_path } => {
            let absolute = remote_workflow_path(shell, workspace_path, task_path)?;
            run_remote_workflow_command(
                config,
                remote_workflow_delete_command(shell, &absolute, false),
            )
        }
    }
}

/// 构造远端 workflow 文档清单命令。
///
/// 适用场景：一次 SSH 读取 root、project root 和直接 task 文档。
/// 例：`/repo -> GSDV-WF 长度帧`。
fn remote_workflow_inventory_command(shell: RemoteShell, workspace_path: &Path) -> String {
    match shell {
        RemoteShell::Posix => {
            let workspace = posix_quote_str(&workspace_path.to_string_lossy());
            format!(
                "w={workspace}; s=\"$w/gsdv-spec\"; emit() {{ f=$1; r=$2; [ -f \"$f\" ] || return 0; pn=$(printf %s \"$r\" | wc -c); cn=$(wc -c <\"$f\"); printf 'GSDV-WF %s %s\\n' \"$pn\" \"$cn\"; printf %s \"$r\"; printf '\\n'; cat \"$f\"; printf '\\n'; }}; emit \"$s/root.md\" 'gsdv-spec/root.md'; p=\"$s/ps\"; if [ -d \"$p\" ]; then for d in \"$p\"/*; do [ -d \"$d\" ] || continue; k=${{d##*/}}; emit \"$d/root.md\" \"gsdv-spec/ps/$k/root.md\"; for f in \"$d\"/task-*.md; do [ -f \"$f\" ] || continue; n=${{f##*/}}; emit \"$f\" \"gsdv-spec/ps/$k/$n\"; done; done; fi"
            )
        }
        RemoteShell::PowerShell | RemoteShell::Cmd => {
            let workspace = powershell_quote(&workspace_path.to_string_lossy());
            let script = format!(
                "$o=[Console]::OpenStandardOutput();function W([string]$s){{$b=[Text.Encoding]::UTF8.GetBytes($s);$o.Write($b,0,$b.Length)}};function E([string]$f,[string]$r){{if([IO.File]::Exists($f)){{$p=[Text.Encoding]::UTF8.GetBytes($r);$b=[IO.File]::ReadAllBytes($f);W(('GSDV-WF '+$p.Length+' '+$b.Length+[char]10));$o.Write($p,0,$p.Length);W([string][char]10);$o.Write($b,0,$b.Length);W([string][char]10)}}}};$w={workspace};$s=Join-Path $w 'gsdv-spec';E (Join-Path $s 'root.md') 'gsdv-spec/root.md';$p=Join-Path $s 'ps';if([IO.Directory]::Exists($p)){{foreach($d in [IO.Directory]::GetDirectories($p)){{$k=[IO.Path]::GetFileName($d);E (Join-Path $d 'root.md') ('gsdv-spec/ps/'+$k+'/root.md');foreach($f in [IO.Directory]::GetFiles($d,'task-*.md')){{$n=[IO.Path]::GetFileName($f);E $f ('gsdv-spec/ps/'+$k+'/'+$n)}}}}}}"
            );
            wrap_powershell_command(shell, script)
        }
    }
}

/// 解析远端 workflow 长度帧为 UTF-8 文档集合。
///
/// 适用场景：文档可包含任意换行，不能用普通行分隔。
/// 例：`GSDV-WF 19 4 + path + body -> map entry`。
fn parse_remote_workflow_inventory(output: &[u8]) -> Result<BTreeMap<PathBuf, String>> {
    let mut cursor = 0usize;
    let mut documents = BTreeMap::new();
    while cursor < output.len() {
        let header_end = output[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| cursor + offset)
            .ok_or_else(|| anyhow!("remote workflow header is incomplete"))?;
        let header = std::str::from_utf8(&output[cursor..header_end])?.trim_end_matches('\r');
        let fields = header.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || fields[0] != "GSDV-WF" {
            bail!("invalid remote workflow header: {header}");
        }
        let path_length = fields[1].parse::<usize>()?;
        let content_length = fields[2].parse::<usize>()?;
        let path_start = header_end + 1;
        let path_end = path_start
            .checked_add(path_length)
            .filter(|end| *end <= output.len())
            .ok_or_else(|| anyhow!("remote workflow path is truncated"))?;
        let path = std::str::from_utf8(&output[path_start..path_end])?;
        let content_start = framed_payload_start(output, path_end, "path")?;
        let content_end = content_start
            .checked_add(content_length)
            .filter(|end| *end <= output.len())
            .ok_or_else(|| anyhow!("remote workflow content is truncated"))?;
        let content = String::from_utf8(output[content_start..content_end].to_vec())
            .with_context(|| format!("remote workflow file is not UTF-8: {path}"))?;
        cursor = framed_payload_start(output, content_end, "content")?;
        let relative = PathBuf::from(path);
        validate_remote_workflow_relative_path(&relative).map_err(anyhow::Error::msg)?;
        if documents.insert(relative, content).is_some() {
            bail!("remote workflow duplicated path: {path}");
        }
    }
    Ok(documents)
}

/// 跳过长度帧中一个 payload 后的换行分隔符。
///
/// 适用场景：同时兼容 LF 和 CRLF 响应。
/// 例：`payload-end + LF -> next-start`。
fn framed_payload_start(output: &[u8], mut cursor: usize, label: &str) -> Result<usize> {
    if output.get(cursor) == Some(&b'\r') {
        cursor += 1;
    }
    if output.get(cursor) != Some(&b'\n') {
        bail!("remote workflow {label} terminator is missing");
    }
    Ok(cursor + 1)
}

/// 把 workspace 相对 WF 路径转换成远端原生绝对路径。
///
/// 适用场景：所有 Remote WF 写操作在构造 shell 命令前统一防穿越。
/// 例：`/repo + gsdv-spec/root.md -> /repo/gsdv-spec/root.md`。
fn remote_workflow_path(
    shell: RemoteShell,
    workspace_path: &Path,
    relative: &Path,
) -> Result<String, String> {
    validate_remote_workflow_relative_path(relative)?;
    let workspace = workspace_path.to_string_lossy();
    if workspace.trim().is_empty() {
        return Err("remote workspace path is empty".to_string());
    }
    let relative = relative.to_string_lossy().replace('\\', "/");
    Ok(match shell {
        RemoteShell::Posix => format!("{}/{}", workspace.trim_end_matches('/'), relative),
        RemoteShell::PowerShell | RemoteShell::Cmd => format!(
            "{}\\{}",
            workspace.trim_end_matches(['/', '\\']),
            relative.replace('/', "\\")
        ),
    })
}

/// 校验 workflow 路径只含普通组件且位于 gsdv-spec 下。
///
/// 适用场景：拒绝 `..`、绝对路径和平台分隔符绕过。
/// 例：`gsdv-spec/../x -> Err`。
fn validate_remote_workflow_relative_path(path: &Path) -> Result<(), String> {
    let mut components = path.components();
    if components.next() != Some(Component::Normal("gsdv-spec".as_ref())) {
        return Err(format!(
            "workflow path is outside gsdv-spec: {}",
            path.display()
        ));
    }
    if components.any(|component| match component {
        Component::Normal(value) => value.to_string_lossy().contains('\\'),
        _ => true,
    }) {
        return Err(format!("workflow path is unsafe: {}", path.display()));
    }
    Ok(())
}

/// 读取一个远端 workflow UTF-8 文件。
///
/// 适用场景：保存或内容 mutation 前读取远端最新 task。
/// 例：`task.md -> String`。
fn read_remote_workflow_file(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    absolute: &str,
) -> Result<String, String> {
    let command = match shell {
        RemoteShell::Posix => {
            let path = posix_quote_str(absolute);
            format!(
                "p={path}; [ -f \"$p\" ] || {{ printf 'workflow file not found: %s\\n' \"$p\" >&2; exit 1; }}; cat \"$p\""
            )
        }
        RemoteShell::PowerShell | RemoteShell::Cmd => {
            let path = powershell_quote(absolute);
            let script = format!(
                "$p={path};if(![IO.File]::Exists($p)){{throw ('workflow file not found: '+$p)}};$b=[IO.File]::ReadAllBytes($p);[Console]::OpenStandardOutput().Write($b,0,$b.Length)"
            );
            wrap_powershell_command(shell, script)
        }
    };
    let output = run_ssh_command(config, &command).map_err(|error| error.to_string())?;
    String::from_utf8(output).map_err(|error| format!("workflow file is not UTF-8: {error}"))
}

/// 原子覆盖一个远端 workflow 文件。
///
/// 适用场景：task 保存和内容 mutation，避免进程中断留下半个 Markdown。
/// 例：`bytes -> 临时文件 -> rename`。
fn write_remote_workflow_file(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    absolute: &str,
    content: &[u8],
) -> Result<(), String> {
    let command = match shell {
        RemoteShell::Posix => {
            let path = posix_quote_str(absolute);
            format!(
                "umask 077; p={path}; d=${{p%/*}}; mkdir -p \"$d\"; t=\"$p.gsdv-$$\"; cat >\"$t\" && mv -f -- \"$t\" \"$p\""
            )
        }
        RemoteShell::PowerShell | RemoteShell::Cmd => {
            let path = powershell_quote(absolute);
            let script = format!(
                "$p={path};$d=[IO.Path]::GetDirectoryName($p);[IO.Directory]::CreateDirectory($d)|Out-Null;$t=$p+'.gsdv-'+$PID;$m=[IO.MemoryStream]::new();[Console]::OpenStandardInput().CopyTo($m);[IO.File]::WriteAllBytes($t,$m.ToArray());Move-Item -LiteralPath $t -Destination $p -Force"
            );
            wrap_powershell_command(shell, script)
        }
    };
    run_ssh_command_with_input(config, &command, content)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// 构造创建空 workflow 文件的命令。
///
/// 适用场景：初始化 root、创建 project 或 task。
/// 例：`allow_existing=true + 已有 root.md -> success`。
fn remote_workflow_create_file_command(
    shell: RemoteShell,
    absolute: &str,
    required_parent: Option<&str>,
    allow_existing: bool,
) -> String {
    match shell {
        RemoteShell::Posix => {
            let path = posix_quote_str(absolute);
            let required = required_parent
                .map(|parent| {
                    let parent = posix_quote_str(parent);
                    format!("r={parent}; [ -d \"$r\" ] || {{ printf 'workflow parent not found: %s\\n' \"$r\" >&2; exit 1; }}; ")
                })
                .unwrap_or_default();
            let existing = if allow_existing {
                "[ -f \"$p\" ] && exit 0; ".to_string()
            } else {
                "[ ! -e \"$p\" ] || { printf 'workflow path already exists: %s\\n' \"$p\" >&2; exit 1; }; ".to_string()
            };
            format!(
                "umask 077; {required}p={path}; {existing}[ ! -e \"$p\" ] || {{ printf 'workflow path is not a file: %s\\n' \"$p\" >&2; exit 1; }}; d=${{p%/*}}; mkdir -p \"$d\"; t=\"$p.gsdv-$$\"; : >\"$t\" && mv -- \"$t\" \"$p\""
            )
        }
        RemoteShell::PowerShell | RemoteShell::Cmd => {
            let path = powershell_quote(absolute);
            let required = required_parent
                .map(|parent| {
                    let parent = powershell_quote(parent);
                    format!("$r={parent};if(![IO.Directory]::Exists($r)){{throw ('workflow parent not found: '+$r)}};")
                })
                .unwrap_or_default();
            let existing = if allow_existing {
                "if([IO.File]::Exists($p)){exit 0};".to_string()
            } else {
                "if([IO.File]::Exists($p) -or [IO.Directory]::Exists($p)){throw ('workflow path already exists: '+$p)};".to_string()
            };
            let script = format!(
                "{required}$p={path};{existing}if([IO.Directory]::Exists($p)){{throw ('workflow path is not a file: '+$p)}};$d=[IO.Path]::GetDirectoryName($p);[IO.Directory]::CreateDirectory($d)|Out-Null;$t=$p+'.gsdv-'+$PID;[IO.File]::WriteAllBytes($t,[byte[]]@());Move-Item -LiteralPath $t -Destination $p"
            );
            wrap_powershell_command(shell, script)
        }
    }
}

/// 构造远端 workflow 路径重命名命令。
///
/// 适用场景：project 目录和 task 文件重命名共享前置检查。
/// 例：`a -> b，b 已存在 -> error`。
fn remote_workflow_rename_command(
    shell: RemoteShell,
    old_path: &str,
    new_path: &str,
    directory: bool,
) -> String {
    match shell {
        RemoteShell::Posix => {
            let old_path = posix_quote_str(old_path);
            let new_path = posix_quote_str(new_path);
            let kind_check = if directory { "-d" } else { "-f" };
            format!(
                "o={old_path}; n={new_path}; [ {kind_check} \"$o\" ] || {{ printf 'workflow source not found: %s\\n' \"$o\" >&2; exit 1; }}; [ ! -e \"$n\" ] || {{ printf 'workflow target already exists: %s\\n' \"$n\" >&2; exit 1; }}; mv -- \"$o\" \"$n\""
            )
        }
        RemoteShell::PowerShell | RemoteShell::Cmd => {
            let old_path = powershell_quote(old_path);
            let new_path = powershell_quote(new_path);
            let source_check = if directory {
                "[IO.Directory]::Exists($o)"
            } else {
                "[IO.File]::Exists($o)"
            };
            let script = format!(
                "$o={old_path};$n={new_path};if(!({source_check})){{throw ('workflow source not found: '+$o)}};if([IO.File]::Exists($n) -or [IO.Directory]::Exists($n)){{throw ('workflow target already exists: '+$n)}};Move-Item -LiteralPath $o -Destination $n"
            );
            wrap_powershell_command(shell, script)
        }
    }
}

/// 构造远端 workflow 路径删除命令。
///
/// 适用场景：删除 project 子树或单个 task。
/// 例：`directory=true -> 递归删除 project`。
fn remote_workflow_delete_command(shell: RemoteShell, absolute: &str, directory: bool) -> String {
    match shell {
        RemoteShell::Posix => {
            let path = posix_quote_str(absolute);
            if directory {
                format!(
                    "p={path}; [ -d \"$p\" ] || {{ printf 'workflow project not found: %s\\n' \"$p\" >&2; exit 1; }}; rm -rf -- \"$p\""
                )
            } else {
                format!(
                    "p={path}; [ -f \"$p\" ] || {{ printf 'workflow task not found: %s\\n' \"$p\" >&2; exit 1; }}; rm -f -- \"$p\""
                )
            }
        }
        RemoteShell::PowerShell | RemoteShell::Cmd => {
            let path = powershell_quote(absolute);
            let (source_check, recurse) = if directory {
                ("[IO.Directory]::Exists($p)", " -Recurse")
            } else {
                ("[IO.File]::Exists($p)", "")
            };
            let script = format!(
                "$p={path};if(!({source_check})){{throw ('workflow path not found: '+$p)}};Remove-Item -LiteralPath $p{recurse} -Force"
            );
            wrap_powershell_command(shell, script)
        }
    }
}

/// 执行一个无 stdin 的远端 workflow 命令。
///
/// 适用场景：创建、重命名和删除只关心退出状态。
/// 例：`exit 0 -> Ok(())`。
fn run_remote_workflow_command(
    config: &RemoteWorkspaceConfig,
    command: String,
) -> Result<(), String> {
    run_ssh_command(config, &command)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// 在 cmd 远端用 PowerShell 执行脚本，PowerShell 远端则直接执行。
///
/// 适用场景：Windows 两种默认 shell 共用二进制安全文件 API。
/// 例：`Cmd + script -> powershell -Command ...`。
fn wrap_powershell_command(shell: RemoteShell, script: String) -> String {
    if shell == RemoteShell::Cmd {
        format!(
            "powershell -NoProfile -NonInteractive -Command \"{}\"",
            script
        )
    } else {
        script
    }
}

/// 通过 SSH 读取一个远端 JSON 文件。
///
/// 适用场景：store 必须存在，sidecar 可以不存在。例：optional 缺失 -> 空对象。
fn read_remote_json(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    relative_path: &str,
    optional: bool,
) -> Result<Value> {
    let command = remote_read_file_command(shell, relative_path, optional);
    let output = run_ssh_command(config, &command)?;
    if optional && output.is_empty() {
        return Ok(serde_json::json!({}));
    }
    serde_json::from_slice(&output).with_context(|| format!("parse remote ~/{relative_path}"))
}

/// 通过 SSH stdin 原子覆盖一个远端 JSON 文件。
///
/// 适用场景：更新 store 或 subagents sidecar。例：Value -> 临时文件后替换。
fn write_remote_json(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    relative_path: &str,
    value: &Value,
) -> Result<()> {
    let content = serde_json::to_vec_pretty(value).context("serialize remote JSON")?;
    let command = remote_write_file_command(shell, relative_path);
    run_ssh_command_with_input(config, &command, &content)?;
    Ok(())
}

/// 一次 SSH 批量检查 workspace 目录并读取全部 sidecar。
///
/// 适用场景：Remote Outline 含多个 workspace。例：7 项 -> 1 次 SSH，而不是 14 次。
fn read_remote_projects_batch(
    config: &RemoteWorkspaceConfig,
    shell: RemoteShell,
    requests: &[RemoteProjectReadRequest],
) -> Result<BTreeMap<usize, RemoteProjectReadResult>> {
    if requests.is_empty() {
        return Ok(BTreeMap::new());
    }
    let command = remote_projects_batch_command(shell, requests);
    let output = run_ssh_command(config, &command)?;
    parse_remote_projects_batch(&output)
}

/// 构造长度前缀批量读取命令，避免 JSON 内容和分隔符冲突。
///
/// 适用场景：POSIX、PowerShell、cmd 共用响应协议。例：header + N bytes + LF。
fn remote_projects_batch_command(
    shell: RemoteShell,
    requests: &[RemoteProjectReadRequest],
) -> String {
    match shell {
        RemoteShell::Posix => requests
            .iter()
            .map(|request| {
                let path = posix_quote_str(&request.path.to_string_lossy());
                let sidecar = posix_quote_str(&request.sidecar_path);
                format!(
                    "d={path}; f=\"$HOME\"/{sidecar}; if [ -d \"$d\" ]; then if [ -f \"$f\" ]; then n=$(wc -c <\"$f\"); printf 'GSDV-BATCH {} 1 %s\\n' \"$n\"; cat \"$f\"; printf '\\n'; else printf 'GSDV-BATCH {} 1 0\\n\\n'; fi; else printf 'GSDV-BATCH {} 0 0\\n\\n'; fi",
                    request.store_index, request.store_index, request.store_index,
                )
            })
            .collect::<Vec<_>>()
            .join("; "),
        RemoteShell::PowerShell | RemoteShell::Cmd => {
            let mut script = "$o=[Console]::OpenStandardOutput(); function W([string]$s){$b=[Text.Encoding]::UTF8.GetBytes($s);$o.Write($b,0,$b.Length)}; ".to_string();
            for request in requests {
                let path = powershell_quote(&request.path.to_string_lossy());
                let sidecar = powershell_quote(&request.sidecar_path);
                script.push_str(&format!(
                    "$d={path};$f=Join-Path $HOME {sidecar};if([IO.Directory]::Exists($d)){{if([IO.File]::Exists($f)){{$b=[IO.File]::ReadAllBytes($f);W(('GSDV-BATCH {} 1 '+$b.Length+[char]10));$o.Write($b,0,$b.Length);W([string][char]10)}}else{{W(('GSDV-BATCH {} 1 0'+[char]10+[char]10))}}}}else{{W(('GSDV-BATCH {} 0 0'+[char]10+[char]10))}};",
                    request.store_index, request.store_index, request.store_index,
                ));
            }
            if shell == RemoteShell::Cmd {
                format!(
                    "powershell -NoProfile -NonInteractive -Command \"{}\"",
                    script
                )
            } else {
                script
            }
        }
    }
}

/// 解析批量响应中的长度前缀和 JSON 字节。
///
/// 适用场景：sidecar 可包含任意换行。例：长度 12 -> 精确读取后续 12 字节。
fn parse_remote_projects_batch(output: &[u8]) -> Result<BTreeMap<usize, RemoteProjectReadResult>> {
    let mut cursor = 0usize;
    let mut results = BTreeMap::new();
    while cursor < output.len() {
        let header_end = output[cursor..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| cursor + offset)
            .ok_or_else(|| anyhow!("remote batch header is incomplete"))?;
        let header = std::str::from_utf8(&output[cursor..header_end])?.trim_end_matches('\r');
        let fields = header.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 4 || fields[0] != "GSDV-BATCH" {
            bail!("invalid remote batch header: {header}");
        }
        let store_index = fields[1].parse::<usize>()?;
        let directory_exists = match fields[2] {
            "1" => true,
            "0" => false,
            value => bail!("invalid remote batch directory flag: {value}"),
        };
        let length = fields[3].parse::<usize>()?;
        let content_start = header_end + 1;
        let content_end = content_start
            .checked_add(length)
            .filter(|end| *end <= output.len())
            .ok_or_else(|| anyhow!("remote batch content is truncated"))?;
        let sidecar = if length == 0 {
            serde_json::json!({})
        } else {
            serde_json::from_slice(&output[content_start..content_end])
                .with_context(|| format!("parse remote sidecar for workspace {store_index}"))?
        };
        cursor = content_end;
        if output.get(cursor) == Some(&b'\r') {
            cursor += 1;
        }
        if output.get(cursor) != Some(&b'\n') {
            bail!("remote batch content terminator is missing");
        }
        cursor += 1;
        if results
            .insert(
                store_index,
                RemoteProjectReadResult {
                    directory_exists,
                    sidecar,
                },
            )
            .is_some()
        {
            bail!("remote batch duplicated workspace {store_index}");
        }
    }
    Ok(results)
}

/// 构造读取远端 HOME 相对文件的 shell 命令。
///
/// 适用场景：不依赖远端 gsdv 或额外工具。例：POSIX store -> cat "$HOME/.gsdv/store"。
fn remote_read_file_command(shell: RemoteShell, relative_path: &str, optional: bool) -> String {
    match shell {
        RemoteShell::Posix => {
            let missing = if optional { ":" } else { "exit 1" };
            format!(
                "p=\"$HOME/{relative_path}\"; if [ -f \"$p\" ]; then cat \"$p\"; else {missing}; fi"
            )
        }
        RemoteShell::PowerShell => powershell_read_file_script(relative_path, optional),
        RemoteShell::Cmd => format!(
            "powershell -NoProfile -NonInteractive -Command \"{}\"",
            powershell_read_file_script(relative_path, optional)
        ),
    }
}

/// 构造写入远端 HOME 相对文件的 shell 命令。
///
/// 适用场景：JSON 从 SSH stdin 无损传输。例：POSIX -> cat 临时文件再 mv。
fn remote_write_file_command(shell: RemoteShell, relative_path: &str) -> String {
    match shell {
        RemoteShell::Posix => format!(
            "umask 077; p=\"$HOME/{relative_path}\"; d=${{p%/*}}; mkdir -p \"$d\"; t=\"$p.gsdv-$$\"; cat >\"$t\" && mv -f -- \"$t\" \"$p\""
        ),
        RemoteShell::PowerShell => powershell_write_file_script(relative_path),
        RemoteShell::Cmd => format!(
            "powershell -NoProfile -NonInteractive -Command \"{}\"",
            powershell_write_file_script(relative_path)
        ),
    }
}

/// 构造 PowerShell 二进制文件读取脚本。
///
/// 适用场景：避免控制台编码破坏 JSON。例：文件存在 -> 原始字节写 stdout。
fn powershell_read_file_script(relative_path: &str, optional: bool) -> String {
    let missing = if optional { "exit 0" } else { "exit 1" };
    format!(
        "$p=Join-Path $HOME {}; if ([IO.File]::Exists($p)) {{ $b=[IO.File]::ReadAllBytes($p); [Console]::OpenStandardOutput().Write($b,0,$b.Length) }} else {{ {missing} }}",
        powershell_quote(relative_path)
    )
}

/// 构造 PowerShell 二进制文件原子写入脚本。
///
/// 适用场景：PowerShell/cmd 远端接收 SSH stdin。例：stdin -> temp -> Move-Item。
fn powershell_write_file_script(relative_path: &str) -> String {
    format!(
        "$p=Join-Path $HOME {}; $d=[IO.Path]::GetDirectoryName($p); [IO.Directory]::CreateDirectory($d) | Out-Null; $t=$p+'.gsdv-'+$PID; $m=[IO.MemoryStream]::new(); [Console]::OpenStandardInput().CopyTo($m); [IO.File]::WriteAllBytes($t,$m.ToArray()); Move-Item -LiteralPath $t -Destination $p -Force",
        powershell_quote(relative_path)
    )
}

/// 通过 SSH 执行远端命令并向其 stdin 写入字节。
///
/// 适用场景：远端 JSON 文件写入。例：content -> ssh child stdin。
fn run_ssh_command_with_input(
    config: &RemoteWorkspaceConfig,
    remote_command: &str,
    input: &[u8],
) -> Result<Vec<u8>> {
    let mut command = Command::new("ssh");
    append_connection_args(&mut command, config);
    let password_path = configure_askpass(&mut command, config)?;
    command
        .arg(ssh_target(config))
        .arg(remote_command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("spawn remote SSH command")?;
    child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("SSH stdin is unavailable"))?
        .write_all(input)
        .context("write remote SSH stdin")?;
    let output = child
        .wait_with_output()
        .context("wait for remote SSH command");
    if let Some(path) = password_path {
        let _ = fs::remove_file(path);
    }
    let output = output?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        bail!(ssh_error_message(&output.stderr))
    }
}

/// 构造远端 Agent 的 shell 命令字符串。
///
/// 适用场景：ssh PTY channel 内启动 Codex/Claude。例：POSIX -> `cd -- ... && exec ...`。
pub fn remote_agent_command(
    shell: RemoteShell,
    workdir: &Path,
    executable: &str,
    args: &[String],
    env: &[(String, String)],
) -> String {
    match shell {
        RemoteShell::Posix => {
            let assignments = env
                .iter()
                .map(|(key, value)| format!("{key}={}", posix_quote_str(value)))
                .collect::<Vec<_>>()
                .join(" ");
            let mut command = format!(
                "cd -- {} && {assignments} exec {}",
                posix_quote(workdir),
                posix_quote_str(executable)
            );
            for arg in args {
                command.push(' ');
                command.push_str(&posix_quote_str(arg));
            }
            command
        }
        RemoteShell::PowerShell => {
            let assignments = env
                .iter()
                .map(|(key, value)| format!("$env:{key}={};", powershell_quote(value)))
                .collect::<Vec<_>>()
                .join(" ");
            let mut command = format!(
                "Set-Location -LiteralPath {}; {assignments} & {}",
                powershell_quote(&workdir.to_string_lossy()),
                powershell_quote(executable)
            );
            for arg in args {
                command.push(' ');
                command.push_str(&powershell_quote(arg));
            }
            command
        }
        RemoteShell::Cmd => {
            let assignments = env
                .iter()
                .map(|(key, value)| format!("set \"{key}={}\" &&", value.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join(" ");
            let mut command = format!(
                "cd /d {} && {assignments} {}",
                cmd_quote(&workdir.to_string_lossy()),
                cmd_quote(executable)
            );
            for arg in args {
                command.push(' ');
                command.push_str(&cmd_quote(arg));
            }
            command
        }
    }
}

/// 打印 askpass 请求的密码并立即退出。
///
/// 适用场景：仅由 OpenSSH 启动的 gsdv 子进程。例：环境变量密码 -> stdout。
pub fn run_askpass_from_env() -> Result<()> {
    if std::env::var_os("GSDV_SSH_ASKPASS").is_none() {
        bail!("SSH askpass marker is missing");
    }
    if let Ok(password) = std::env::var("GSDV_SSH_PASSWORD") {
        std::io::stdout()
            .write_all(password.as_bytes())
            .context("write SSH password")?;
        return std::io::stdout().flush().context("flush SSH password");
    }
    let password_path = std::env::var_os("GSDV_SSH_PASSWORD_FILE")
        .context("SSH password file and value are missing")?;
    let mut password = Vec::new();
    fs::File::open(password_path)
        .context("open temporary SSH password")?
        .read_to_end(&mut password)
        .context("read temporary SSH password")?;
    std::io::stdout()
        .write_all(&password)
        .context("write SSH password")?;
    std::io::stdout().flush().context("flush SSH password")
}

/// 设置或移除 JSON 对象中的可选字符串字段。
///
/// 适用场景：远端主 agent 配置回写。例：None -> JSON null。
fn set_optional_json_string(root: &mut Value, key: &str, value: Option<&str>) {
    root[key] = value.map(Value::from).unwrap_or(Value::Null);
}

/// 从 store workspace 和 sidecar 构造一个远端项目快照。
///
/// 适用场景：SSH 读取结果的单项转换。例：缺 sidecar -> 默认单列 main。
fn project_snapshot(
    store_index: usize,
    path: PathBuf,
    workspace: &Value,
    sidecar: &Value,
) -> Result<RemoteProjectSnapshot> {
    let agent_kind = workspace
        .get("agent_kind")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    let subagents: Vec<SubagentViewData> = sidecar
        .get("subagents")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    let mut rows: Vec<AgentRowViewData> = sidecar
        .get("rows")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    if rows.is_empty() {
        let legacy_columns: Vec<AgentColumnViewData> = sidecar
            .get("columns")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();
        let columns = if legacy_columns.is_empty() {
            vec![AgentColumnViewData {
                id: "main".to_string(),
                tabs: vec![crate::gui::data::AgentColumnSlot::Main],
                active_slot: crate::gui::data::AgentColumnSlot::Main,
                width_weight: 1.0,
                collapsed: false,
            }]
        } else {
            legacy_columns
        };
        rows.push(AgentRowViewData {
            columns,
            height_weight: 1.0,
            collapsed: false,
        });
    }
    let focus = sidecar
        .get("focus")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?;
    let (rows, focus) = crate::gui::data::normalize_agent_rows(rows, focus, &subagents);
    Ok(RemoteProjectSnapshot {
        store_index,
        path: path.clone(),
        agent_kind,
        agent_model: string_field(workspace, "agent_model"),
        agent_model_provider: string_field(workspace, "agent_model_provider"),
        agent_effort: string_field(workspace, "agent_effort"),
        agent_fast_mode: workspace.get("agent_fast_mode").and_then(Value::as_bool),
        agent_work_dir: string_field(workspace, "agent_work_dir").map(PathBuf::from),
        agent_id: string_field(workspace, "agent_id")
            .unwrap_or_else(|| format!("remote-{}", crate::gui::data::workspace_store_key(&path))),
        session_id: string_field(workspace, "session_id"),
        subagents,
        rows,
        focus,
    })
}

/// 保留最新 sidecar 中按 agent ID 匹配的 session ID。
///
/// 适用场景：布局保存与远端 hook 并发。例：载荷 session=None -> 使用远端现值。
fn preserve_session_ids(latest: &Value, subagents: &mut [SubagentViewData]) {
    let Some(entries) = latest.get("subagents").and_then(Value::as_array) else {
        return;
    };
    for subagent in subagents {
        let session = entries.iter().find_map(|entry| {
            (entry.get("agent_id").and_then(Value::as_str) == Some(subagent.agent_id.as_str()))
                .then(|| entry.get("session_id").and_then(Value::as_str))
                .flatten()
        });
        if let Some(session) = session {
            subagent.session_id = Some(session.to_string());
        }
    }
}

/// 读取 JSON 字符串字段并过滤空值。
///
/// 适用场景：兼容旧版远端 store。例：字段缺失 -> None。
fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 将 OpenSSH stderr 整理成用户可见错误。
///
/// 适用场景：连接或远端命令失败。例：空 stderr -> 通用错误。
fn ssh_error_message(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_string();
    if message.is_empty() {
        "SSH command failed".to_string()
    } else {
        message
    }
}

/// POSIX shell 单引号转义路径。
///
/// 适用场景：远端目录包含空格或引号。例：`a'b` -> `'a'\"'\"'b'`。
fn posix_quote(path: &Path) -> String {
    posix_quote_str(&path.to_string_lossy())
}

/// POSIX shell 单引号转义字符串。
///
/// 适用场景：Agent 参数安全拼接。例：空格参数 -> 单个 shell token。
fn posix_quote_str(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// PowerShell 单引号转义字符串。
///
/// 适用场景：LiteralPath 和 Agent 参数。例：`a'b` -> `'a''b'`。
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// cmd.exe 双引号转义字符串。
///
/// 适用场景：Windows 路径和 Agent 参数。例：空格路径 -> 双引号 token。
fn cmd_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
