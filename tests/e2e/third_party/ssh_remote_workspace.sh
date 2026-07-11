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
    if [[ -n "${temporary_dir:-}" ]]; then
        rm -rf "$temporary_dir"
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
printf 'ok: shell=%s connections=isolated json=roundtrip\n' "$remote_shell"
