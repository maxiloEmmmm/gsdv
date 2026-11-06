#!/usr/bin/env bash
# Runs an isolated desktop app under Xvfb and verifies spotlight plus Codex hooks.
# Example: bash tests/e2e/third_party/spotlight_mode.sh target/debug/gsdv 20179
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
binary="${1:-$repo_root/target/debug/gsdv}"
binary="$(realpath "$binary")"
remote_port="${2:-20179}"
tmp_dir="$(mktemp -d)"
test_home="$tmp_dir/home"
workspace="$tmp_dir/workspace"
store="$tmp_dir/store.json"
app_log="$tmp_dir/gsdv.log"
marker="$tmp_dir/socket-marker"
app_pid=""
test_status=0

# Prints isolated diagnostics only when the offscreen check fails.
on_error() {
  test_status=$?
  find "$tmp_dir" -path '*/gsdv-screenshots/*.png' -type f -exec cp {} /tmp/gsdv-offscreen-failed.png \; 2>/dev/null || true
  printf 'offscreen app log:\n' >&2
  tail -n 80 "$app_log" >&2 || true
  printf 'offscreen screenshot files:\n' >&2
  find "$tmp_dir" -path '*/gsdv-screenshots/*' -type f -print >&2 || true
  exit "$test_status"
}

# Stops only this test's Xvfb/app process and removes its isolated files.
cleanup() {
  if [ -n "$app_pid" ]; then
    kill "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  rm -rf "$tmp_dir"
}
trap cleanup EXIT
trap on_error ERR

mkdir -p "$test_home/bin" "$workspace" "$workspace/target/gsdv-screenshots"
touch "$marker"

cat >"$test_home/bin/codex" <<'SH'
#!/bin/sh
printf 'isolated spotlight agent\n'
while IFS= read -r line; do
  printf 'input: %s\n' "$line"
done
SH
chmod +x "$test_home/bin/codex"

jq -n \
  --arg path "$workspace" \
  --argjson port "$remote_port" \
  '{
    active: 0,
    spotlight_agent_ids: ["fixture-agent"],
    runtime_settings: {
      remote_server_enabled: true,
      remote_server_host: "127.0.0.1",
      remote_server_port: $port
    },
    workspaces: [{
      path: $path,
      agent_kind: "codex",
      agent_id: "fixture-agent"
    }]
  }' >"$store"

(
  cd "$workspace"
  export E2E_HOME="$test_home"
  export E2E_CONFIG_HOME="$test_home/.config"
  export E2E_STORE="$store"
  export E2E_PATH="$test_home/bin:$PATH"
  export E2E_BINARY="$binary"
  exec env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET \
    WINIT_UNIX_BACKEND=x11 \
    XDG_SESSION_TYPE=x11 \
    xvfb-run -a bash -c '
      env \
        HOME="$E2E_HOME" \
        XDG_CONFIG_HOME="$E2E_CONFIG_HOME" \
        GSDV_STORE_PATH="$E2E_STORE" \
        GSDV_SCREENSHOT_REQUEST_POLL=1 \
        PATH="$E2E_PATH" \
        "$E2E_BINARY" &
      child=$!
      trap '\''kill "$child" 2>/dev/null || true; wait "$child" 2>/dev/null || true'\'' EXIT
      trap '\''exit 130'\'' INT
      trap '\''exit 143'\'' TERM
      while kill -0 "$child" 2>/dev/null; do
        xdotool mousemove 10 10 2>/dev/null || true
        xdotool mousemove 11 10 2>/dev/null || true
        sleep 0.2
      done
      wait "$child"
    '
) >"$app_log" 2>&1 &
app_pid="$!"

# Waits for the isolated remote server before sending status hooks.
for _ in $(seq 1 160); do
  if curl --fail --silent "http://127.0.0.1:$remote_port/api/workspace" >/dev/null 2>&1; then
    break
  fi
  sleep 0.05
done
curl --fail --silent "http://127.0.0.1:$remote_port/api/workspace" >/dev/null

hook_endpoint=""
for _ in $(seq 1 100); do
  for candidate in /tmp/gsdv-hook-*.sock; do
    if [ -S "$candidate" ] && [ "$candidate" -nt "$marker" ]; then
      hook_endpoint="$candidate"
      break
    fi
  done
  [ -n "$hook_endpoint" ] && break
  sleep 0.05
done
[ -n "$hook_endpoint" ]
hook_script="$test_home/.gsdv/hooks/agent-status-hook"
[ -x "$hook_script" ]

# Sends one real installed hook payload to the isolated app socket.
send_hook() {
  local payload="$1"
  printf '%s' "$payload" | env \
    GSDV_AGENT_ID=fixture-agent \
    GSDV_WORKSPACE_DIR="$workspace" \
    GSDV_HOOK_ENDPOINT="$hook_endpoint" \
    "$hook_script"
}

# Waits until the remote metadata API exposes the expected Agent activity.
wait_activity() {
  local expected="$1"
  for _ in $(seq 1 80); do
    if curl --fail --silent "http://127.0.0.1:$remote_port/api/workspace" | jq -e \
      --arg expected "$expected" \
      '[.workspaces[].rows[].cols[].agents[] | select(.agent_id == "fixture-agent")][0]
       | .in_spotlight == true and .activity == $expected' >/dev/null; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

send_hook '{"hook_event_name":"PreModelRequest","session_id":"fixture-session"}'
wait_activity busy
send_hook '{"hook_event_name":"PostModelRequest","status":"failed","error":"HTTP 503"}'
wait_activity error
send_hook '{"hook_event_name":"Stop","session_id":"fixture-session"}'
wait_activity idle

jq -e '.hooks.PreModelRequest and .hooks.PostModelRequest' \
  "$test_home/.codex/hooks.json" >/dev/null

printf 'spotlight\n' >"$workspace/target/gsdv-screenshots/capture.request"
latest="$workspace/target/gsdv-screenshots/latest.txt"
for _ in $(seq 1 120); do
  [ -s "$latest" ] && break
  sleep 0.05
done
screenshot="$(cat "$latest")"
[ -s "$screenshot" ]
identify -format '%w %h' "$screenshot" | awk '{exit !($1 >= 640 && $2 >= 480)}'
magick "$screenshot" -crop 244x720+0+0 txt:- | rg '#F5B800|#F5B84B' >/dev/null

printf 'PASS: offscreen spotlight render, persisted membership, and Codex model hooks\n'
