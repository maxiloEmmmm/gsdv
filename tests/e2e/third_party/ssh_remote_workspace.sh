#!/usr/bin/env bash
# 验证 gsdv Remote Workspace 依赖的真实 OpenSSH 行为。
#
# 场景：系统 ssh 必须支持并行独立连接、自动 host key、keepalive 参数，以及
# 通过远端 stdin 无损读写临时 JSON。脚本不会修改真实 store/sidecar，也不会
# 保存密码；密码仅从 GSDV_E2E_SSH_PASSWORD 环境变量读取。
#
# 示例：
# GSDV_E2E_SSH_HOST=host GSDV_E2E_SSH_USER=user \
# GSDV_E2E_SSH_PASSWORD=secret ./tests/e2e/third_party/ssh_remote_workspace.sh

set -euo pipefail

# OpenSSH 以 SSH_ASKPASS 调用本脚本时，只输出环境变量密码。
# 示例：GSDV_E2E_ASKPASS_MODE=1 -> stdout password。
if [[ "${GSDV_E2E_ASKPASS_MODE:-}" == "1" ]]; then
    printf '%s' "${GSDV_E2E_SSH_PASSWORD:?missing password}"
    exit 0
fi

# 校验必需环境变量并返回其值。
# 示例：require_env GSDV_E2E_SSH_HOST -> host。
require_env() {
    local name="$1"
    local value="${!name:-}"
    if [[ -z "$value" ]]; then
        printf 'missing required environment variable: %s\n' "$name" >&2
        exit 2
    fi
    printf '%s' "$value"
}

# 删除本机临时目录。
# 示例：脚本正常退出或失败 -> 不遗留临时输出。
cleanup() {
    if [[ -n "${remote_workflow_relative:-}" && -n "${remote_shell:-}" ]]; then
        case "$remote_shell" in
            posix)
                ssh "${common_args[@]}" "$target" \
                    "r=\"\$HOME/$remote_workflow_relative\"; rm -rf -- \"\$r\"" || true
                ;;
            powershell | cmd)
                remote_powershell \
                    "\$r=Join-Path \$HOME '$remote_workflow_relative';if([IO.Directory]::Exists(\$r)){Remove-Item -LiteralPath \$r -Recurse -Force}" || true
                ;;
        esac
    fi
    if [[ -n "${temporary_dir:-}" ]]; then
        rm -rf "$temporary_dir"
    fi
}

# 在 PowerShell/cmd 远端执行同一段 PowerShell 文件 API 脚本。
# 示例：remote_powershell '$HOME' -> 输出远端 HOME。
remote_powershell() {
    local script="$1"
    if [[ "$remote_shell" == "cmd" ]]; then
        ssh "${common_args[@]}" "$target" \
            "powershell -NoProfile -NonInteractive -Command \"$script\""
    else
        ssh "${common_args[@]}" "$target" "$script"
    fi
}

host="$(require_env GSDV_E2E_SSH_HOST)"
user="$(require_env GSDV_E2E_SSH_USER)"
port="${GSDV_E2E_SSH_PORT:-22}"
target="${user}@${host}"
temporary_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-ssh-e2e.XXXXXX")"
trap cleanup EXIT

common_args=(
    -p "$port"
    -o ConnectTimeout=5
    -o ServerAliveInterval=10
    -o ServerAliveCountMax=3
    -o StrictHostKeyChecking=accept-new
)

if [[ -n "${GSDV_E2E_SSH_PRIVATE_KEY:-}" ]]; then
    common_args+=(
        -o BatchMode=yes
        -o IdentitiesOnly=yes
        -i "$GSDV_E2E_SSH_PRIVATE_KEY"
    )
elif [[ -n "${GSDV_E2E_SSH_PASSWORD:-}" ]]; then
    export SSH_ASKPASS="$0"
    export SSH_ASKPASS_REQUIRE=force
    export DISPLAY=gsdv-ssh-e2e
    export GSDV_E2E_ASKPASS_MODE=1
    common_args+=(
        -o PreferredAuthentications=password
        -o NumberOfPasswordPrompts=1
    )
else
    common_args+=(-o BatchMode=yes)
fi

ssh "${common_args[@]}" "$target" \
    'printf channel-a; sleep 1; printf -- "-done"' >"$temporary_dir/a" &
channel_a_pid=$!
ssh "${common_args[@]}" "$target" \
    'printf channel-b; sleep 1; printf -- "-done"' >"$temporary_dir/b" &
channel_b_pid=$!
wait "$channel_a_pid"
wait "$channel_b_pid"
[[ "$(<"$temporary_dir/a")" == "channel-a-done" ]]
[[ "$(<"$temporary_dir/b")" == "channel-b-done" ]]

comspec="$(ssh "${common_args[@]}" "$target" 'echo %COMSPEC%')"
if [[ -n "$comspec" && "$comspec" != *'%COMSPEC%'* ]]; then
    remote_shell=cmd
else
    pshome="$(ssh "${common_args[@]}" "$target" 'echo $PSHOME')"
    if [[ -n "$pshome" && "$pshome" != '$PSHOME' ]]; then
        remote_shell=powershell
    else
        [[ "$(ssh "${common_args[@]}" "$target" 'printf gsdv-posix')" == "gsdv-posix" ]]
        remote_shell=posix
    fi
fi

if [[ "$remote_shell" == "posix" ]]; then
    remote_home="$(ssh "${common_args[@]}" "$target" 'printf %s "$HOME"')"
    ssh "${common_args[@]}" "$target" \
        "test -d '$remote_home'"
fi

payload='{"gsdv":"remote-workspace-e2e"}'
case "$remote_shell" in
    posix)
        roundtrip="$(printf '%s' "$payload" | ssh "${common_args[@]}" "$target" \
            'umask 077; p="$HOME/.gsdv/.remote-workspace-e2e-$$"; cat >"$p"; cat "$p"; rm -f "$p"')"
        ;;
    powershell)
        roundtrip="$(printf '%s' "$payload" | ssh "${common_args[@]}" "$target" \
            '$p=Join-Path $HOME ".gsdv/.remote-workspace-e2e"; $i=[Console]::OpenStandardInput(); $f=[IO.File]::Create($p); $i.CopyTo($f); $f.Dispose(); [Console]::OpenStandardOutput().Write([IO.File]::ReadAllBytes($p)); Remove-Item -LiteralPath $p')"
        ;;
    cmd)
        roundtrip="$(printf '%s' "$payload" | ssh "${common_args[@]}" "$target" \
            'powershell -NoProfile -NonInteractive -Command "$p=Join-Path $HOME ''.gsdv/.remote-workspace-e2e''; $i=[Console]::OpenStandardInput(); $f=[IO.File]::Create($p); $i.CopyTo($f); $f.Dispose(); [Console]::OpenStandardOutput().Write([IO.File]::ReadAllBytes($p)); Remove-Item -LiteralPath $p"')"
        ;;
esac

[[ "$roundtrip" == "$payload" ]]

remote_workflow_relative=".gsdv/remote workflow e2e-$$"
workflow_payload=$'task desc\n\n## [ ] build\nfirst body\n'
case "$remote_shell" in
    posix)
        ssh "${common_args[@]}" "$target" \
            "r=\"\$HOME/$remote_workflow_relative\"; rm -rf -- \"\$r\"; d=\"\$r/gsdv-spec/ps/demo\"; mkdir -p \"\$d\"; : >\"\$r/gsdv-spec/root.md\"; : >\"\$d/root.md\"; printf '%s' 'task desc

## [ ] build
first body
' >\"\$d/task-a.md\""
        ssh "${common_args[@]}" "$target" \
            "w=\"\$HOME/$remote_workflow_relative\"; s=\"\$w/gsdv-spec\"; emit() { f=\$1; r=\$2; [ -f \"\$f\" ] || return 0; pn=\$(printf %s \"\$r\" | wc -c); cn=\$(wc -c <\"\$f\"); printf 'GSDV-WF %s %s\\n' \"\$pn\" \"\$cn\"; printf %s \"\$r\"; printf '\\n'; cat \"\$f\"; printf '\\n'; }; emit \"\$s/root.md\" 'gsdv-spec/root.md'; p=\"\$s/ps\"; for d in \"\$p\"/*; do [ -d \"\$d\" ] || continue; k=\${d##*/}; emit \"\$d/root.md\" \"gsdv-spec/ps/\$k/root.md\"; for f in \"\$d\"/task-*.md; do [ -f \"\$f\" ] || continue; n=\${f##*/}; emit \"\$f\" \"gsdv-spec/ps/\$k/\$n\"; done; done" \
            >"$temporary_dir/workflow-inventory"
        grep -aFq 'gsdv-spec/ps/demo/task-a.md' "$temporary_dir/workflow-inventory"
        workflow_roundtrip="$(printf '%s' "$workflow_payload" | ssh "${common_args[@]}" "$target" \
            "p=\"\$HOME/$remote_workflow_relative/gsdv-spec/ps/demo/task-a.md\"; d=\${p%/*}; mkdir -p \"\$d\"; t=\"\$p.gsdv-\$\$\"; cat >\"\$t\" && mv -f -- \"\$t\" \"\$p\"; cat \"\$p\"")"
        [[ "$workflow_roundtrip" == "$workflow_payload" ]]
        ssh "${common_args[@]}" "$target" \
            "r=\"\$HOME/$remote_workflow_relative/gsdv-spec/ps\"; mv -- \"\$r/demo/task-a.md\" \"\$r/demo/task-b.md\"; mv -- \"\$r/demo\" \"\$r/renamed\"; rm -f -- \"\$r/renamed/task-b.md\"; rm -rf -- \"\$r/renamed\"; [ ! -e \"\$r/renamed\" ]"
        ;;
    powershell | cmd)
        remote_powershell \
            "\$r=Join-Path \$HOME '$remote_workflow_relative';if([IO.Directory]::Exists(\$r)){Remove-Item -LiteralPath \$r -Recurse -Force};\$d=Join-Path \$r 'gsdv-spec/ps/demo';[IO.Directory]::CreateDirectory(\$d)|Out-Null;[IO.File]::WriteAllBytes((Join-Path \$r 'gsdv-spec/root.md'),[byte[]]@());[IO.File]::WriteAllBytes((Join-Path \$d 'root.md'),[byte[]]@());\$b=[Text.Encoding]::UTF8.GetBytes('task desc'+[char]10+[char]10+'## [ ] build'+[char]10+'first body'+[char]10);[IO.File]::WriteAllBytes((Join-Path \$d 'task-a.md'),\$b)"
        remote_powershell \
            "\$o=[Console]::OpenStandardOutput();function W([string]\$s){\$b=[Text.Encoding]::UTF8.GetBytes(\$s);\$o.Write(\$b,0,\$b.Length)};function E([string]\$f,[string]\$r){if([IO.File]::Exists(\$f)){\$p=[Text.Encoding]::UTF8.GetBytes(\$r);\$b=[IO.File]::ReadAllBytes(\$f);W(('GSDV-WF '+\$p.Length+' '+\$b.Length+[char]10));\$o.Write(\$p,0,\$p.Length);W([string][char]10);\$o.Write(\$b,0,\$b.Length);W([string][char]10)}};\$r=Join-Path \$HOME '$remote_workflow_relative';\$s=Join-Path \$r 'gsdv-spec';E (Join-Path \$s 'root.md') 'gsdv-spec/root.md';\$p=Join-Path \$s 'ps';foreach(\$d in [IO.Directory]::GetDirectories(\$p)){\$k=[IO.Path]::GetFileName(\$d);E (Join-Path \$d 'root.md') ('gsdv-spec/ps/'+\$k+'/root.md');foreach(\$f in [IO.Directory]::GetFiles(\$d,'task-*.md')){\$n=[IO.Path]::GetFileName(\$f);E \$f ('gsdv-spec/ps/'+\$k+'/'+\$n)}}" \
            >"$temporary_dir/workflow-inventory"
        grep -aFq 'gsdv-spec/ps/demo/task-a.md' "$temporary_dir/workflow-inventory"
        workflow_roundtrip="$(printf '%s' "$workflow_payload" | remote_powershell \
            "\$p=Join-Path \$HOME '$remote_workflow_relative/gsdv-spec/ps/demo/task-a.md';\$d=[IO.Path]::GetDirectoryName(\$p);[IO.Directory]::CreateDirectory(\$d)|Out-Null;\$t=\$p+'.gsdv-'+\$PID;\$m=[IO.MemoryStream]::new();[Console]::OpenStandardInput().CopyTo(\$m);[IO.File]::WriteAllBytes(\$t,\$m.ToArray());Move-Item -LiteralPath \$t -Destination \$p -Force;\$b=[IO.File]::ReadAllBytes(\$p);[Console]::OpenStandardOutput().Write(\$b,0,\$b.Length)")"
        [[ "$workflow_roundtrip" == "$workflow_payload" ]]
        remote_powershell \
            "\$r=Join-Path \$HOME '$remote_workflow_relative/gsdv-spec/ps';Move-Item -LiteralPath (Join-Path \$r 'demo/task-a.md') -Destination (Join-Path \$r 'demo/task-b.md');Move-Item -LiteralPath (Join-Path \$r 'demo') -Destination (Join-Path \$r 'renamed');Remove-Item -LiteralPath (Join-Path \$r 'renamed/task-b.md') -Force;Remove-Item -LiteralPath (Join-Path \$r 'renamed') -Recurse -Force;if([IO.Directory]::Exists((Join-Path \$r 'renamed'))){exit 1}"
        ;;
esac

printf 'ok: shell=%s connections=isolated json=roundtrip workflow=crud\n' "$remote_shell"
