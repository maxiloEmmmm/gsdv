#!/usr/bin/env bash
# Verifies that TUI proxy settings persist and restart open Agent PTYs with the new environment.
# Example: bash tests/e2e/third_party/tui_proxy_settings.sh target/release/gsdv
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
target_dir="$(cd "$repo_root" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
binary="${1:-$target_dir/release/gsdv}"
binary="$(realpath "$binary")"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-tui-proxy.XXXXXX")"
test_home="$tmp_dir/home"
workspace="$tmp_dir/workspace"
store="$tmp_dir/store.json"
env_log="$tmp_dir/agent-env.log"
socket_name="gsdv-tui-proxy-$$"
session_name="proxy"
target="$session_name:0.0"

# Stops only this test's isolated tmux server and process tree.
cleanup() {
  pane_pid="$(tmux -L "$socket_name" list-panes -t "$target" -F '#{pane_pid}' 2>/dev/null | head -n 1 || true)"
  if [[ -n "$pane_pid" ]]; then
    pkill -TERM -P "$pane_pid" 2>/dev/null || true
    kill "$pane_pid" 2>/dev/null || true
  fi
  tmux -L "$socket_name" kill-server >/dev/null 2>&1 || true
  rm -rf -- "$tmp_dir"
}
trap cleanup EXIT

mkdir -p "$test_home/bin" "$workspace"
cat >"$test_home/bin/codex" <<'SH'
#!/bin/sh
printf 'all_proxy=%s|http_proxy=%s|https_proxy=%s|no_proxy=%s\n' \
  "${all_proxy-}" "${http_proxy-}" "${https_proxy-}" "${no_proxy-}" >>"$E2E_ENV_LOG"
printf 'proxy settings fixture ready\n'
while :; do
  sleep 1
done
SH
chmod +x "$test_home/bin/codex"

jq -n \
  --arg path "$workspace" \
  '{
    active: 0,
    default_agent_kind: "codex",
    runtime_settings: {remote_server_enabled: false},
    workspaces: [{
      path: $path,
      agent_kind: "codex",
      agent_id: "proxy-settings-agent"
    }]
  }' >"$store"

tmux -L "$socket_name" new-session -d -x 120 -y 40 -s "$session_name" \
  "cd '$workspace' && exec env HOME='$test_home' GSDV_STORE_PATH='$store' PATH='$test_home/bin:$PATH' E2E_ENV_LOG='$env_log' all_proxy= http_proxy= https_proxy= no_proxy= '$binary' tui --fps 60"

for _ in $(seq 1 300); do
  if [[ -s "$env_log" ]] && tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '[Settings]' >/dev/null; then
    break
  fi
  sleep 0.02
done
[[ -s "$env_log" ]] || {
  echo 'FAIL: fake Agent did not start' >&2
  exit 1
}
tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '[Settings]' >/dev/null || {
  echo 'FAIL: Settings button was not rendered' >&2
  exit 1
}

# Click Settings in the main bottom bar and verify the first settings surface.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;66;40M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;66;40m'
for _ in $(seq 1 200); do
  pane="$(tmux -L "$socket_name" capture-pane -p -t "$target")"
  if rg -F 'Network Proxy' <<<"$pane" >/dev/null && rg -F 'Additional no_proxy' <<<"$pane" >/dev/null; then
    break
  fi
  sleep 0.02
done
pane="$(tmux -L "$socket_name" capture-pane -p -t "$target")"
for label in 'Network Proxy' 'Enable proxy' 'Proxy URL' 'Mirror protocol' 'Additional no_proxy'; do
  rg -F "$label" <<<"$pane" >/dev/null || {
    printf 'FAIL: proxy form is missing %s\n%s\n' "$label" "$pane" >&2
    exit 1
  }
done

# Enable proxy, fill both text values, enable protocol mirroring, then submit.
tmux -L "$socket_name" send-keys -t "$target" Right
tmux -L "$socket_name" send-keys -t "$target" Tab
tmux -L "$socket_name" send-keys -t "$target" -l 'socks5://127.0.0.1:7891'
tmux -L "$socket_name" send-keys -t "$target" Tab Right Tab
tmux -L "$socket_name" send-keys -t "$target" -l '.internal,10.0.0.0/8'
tmux -L "$socket_name" send-keys -t "$target" Enter

for _ in $(seq 1 400); do
  if [[ "$(wc -l <"$env_log")" -ge 2 ]] \
    && jq -e '
      .network_settings.proxy_enabled == true and
      .network_settings.proxy == "socks5://127.0.0.1:7891" and
      .network_settings.no_proxy == ".internal,10.0.0.0/8" and
      .network_settings.mirror_proxy_protocol == true
    ' "$store" >/dev/null 2>&1; then
    break
  fi
  sleep 0.025
done

jq -e '
  .network_settings.proxy_enabled == true and
  .network_settings.proxy == "socks5://127.0.0.1:7891" and
  .network_settings.no_proxy == ".internal,10.0.0.0/8" and
  .network_settings.mirror_proxy_protocol == true
' "$store" >/dev/null || {
  echo 'FAIL: TUI proxy settings were not persisted' >&2
  jq '.network_settings' "$store" >&2 || true
  exit 1
}

[[ "$(wc -l <"$env_log")" -ge 2 ]] || {
  echo 'FAIL: open Agent did not restart after proxy settings changed' >&2
  exit 1
}
last_env="$(tail -n 1 "$env_log")"
[[ "$last_env" == 'all_proxy=socks5://127.0.0.1:7891|http_proxy=http://127.0.0.1:7891|https_proxy=http://127.0.0.1:7891|no_proxy=192.168.0.0/16,127.0.0.1/8,.internal,10.0.0.0/8' ]] || {
  printf 'FAIL: restarted Agent received wrong proxy environment\n%s\n' "$last_env" >&2
  exit 1
}

printf 'PASS: TUI proxy settings persisted and restarted the Agent with the new environment\n'
