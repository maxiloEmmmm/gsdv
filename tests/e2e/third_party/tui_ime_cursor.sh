#!/usr/bin/env bash
# Verifies that hidden Agent cursor movement cannot move the outer TUI IME anchor.
# Example: bash tests/e2e/third_party/tui_ime_cursor.sh target/release/gsdv
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
target_dir="$(cd "$repo_root" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
binary="${1:-$target_dir/release/gsdv}"
binary="$(realpath "$binary")"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-tui-ime.XXXXXX")"
test_home="$tmp_dir/home"
workspace="$tmp_dir/workspace"
store="$tmp_dir/store.json"
ready_file="$tmp_dir/agent.ready"
move_file="$tmp_dir/agent.move"
done_file="$tmp_dir/agent.done"
raw_output="$tmp_dir/tui-output.bin"
socket_name="gsdv-tui-ime-$$"
sync_socket_name="gsdv-tui-sync-$$"
session_name="ime"
target="$session_name:0.0"
sync_ready_file="$tmp_dir/sync.ready"
sync_release_file="$tmp_dir/sync.release"

# Stops only the isolated tmux server and its GSDV process tree.
cleanup() {
  pane_pid="$(tmux -L "$socket_name" list-panes -t "$target" -F '#{pane_pid}' 2>/dev/null | head -n 1 || true)"
  if [[ -n "$pane_pid" ]]; then
    pkill -TERM -P "$pane_pid" 2>/dev/null || true
    kill "$pane_pid" 2>/dev/null || true
  fi
  tmux -L "$socket_name" kill-server >/dev/null 2>&1 || true
  tmux -L "$sync_socket_name" kill-server >/dev/null 2>&1 || true
  rm -rf -- "$tmp_dir"
}
trap cleanup EXIT

mkdir -p "$test_home/bin" "$workspace"

tmux -L "$sync_socket_name" new-session -d -x 80 -y 24 -s sync \
  "printf '\033[?2026h'; touch '$sync_ready_file'; while [ ! -f '$sync_release_file' ]; do sleep 0.01; done; printf '\033[?2026l'; sleep 1"
for _ in $(seq 1 200); do
  [[ -f "$sync_ready_file" ]] && break
  sleep 0.01
done
[[ -f "$sync_ready_file" ]] || {
  echo "FAIL: tmux synchronized-output probe did not start" >&2
  exit 1
}
sync_flag="$(tmux -L "$sync_socket_name" display-message -p -t sync:0.0 '#{synchronized_output_flag}')"
[[ "$sync_flag" == "1" ]] || {
  echo "FAIL: tmux $(tmux -V) does not honor application synchronized output" >&2
  exit 1
}
touch "$sync_release_file"
for _ in $(seq 1 200); do
  sync_flag="$(tmux -L "$sync_socket_name" display-message -p -t sync:0.0 '#{synchronized_output_flag}')"
  [[ "$sync_flag" == "0" ]] && break
  sleep 0.01
done
[[ "$sync_flag" == "0" ]] || {
  echo "FAIL: tmux synchronized-output mode did not close" >&2
  exit 1
}
tmux -L "$sync_socket_name" kill-server >/dev/null 2>&1 || true

cat >"$test_home/bin/codex" <<'SH'
#!/bin/sh
printf '\033[2J\033[H\033[?25h\033[20;8H'
printf 'ready'
printf '\033[20;8H'
printf '{"hook_event_name":"PreModelRequest","session_id":"ime-e2e"}' \
  | "$E2E_GSDV_BINARY" agent-status-hook
touch "$E2E_READY_FILE"
while [ ! -f "$E2E_MOVE_FILE" ]; do
  sleep 0.01
done
index=1
while [ "$index" -le 80 ]; do
  row=$((2 + index % 14))
  column=$((35 + index % 30))
  printf '\033[?25l\033[%s;%sHhidden-%03d' "$row" "$column" "$index"
  sleep 0.02
  index=$((index + 1))
done
touch "$E2E_DONE_FILE"
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
      agent_id: "ime-fixture-agent"
    }]
  }' >"$store"

tmux -L "$socket_name" new-session -d -x 120 -y 40 -s "$session_name" \
  "cd '$workspace' && exec env HOME='$test_home' GSDV_STORE_PATH='$store' PATH='$test_home/bin:$PATH' E2E_GSDV_BINARY='$binary' E2E_READY_FILE='$ready_file' E2E_MOVE_FILE='$move_file' E2E_DONE_FILE='$done_file' '$binary' tui --fps 60"
tmux -L "$socket_name" pipe-pane -O -t "$target" "cat >'$raw_output'"

for _ in $(seq 1 300); do
  [[ -f "$ready_file" ]] && break
  sleep 0.02
done
[[ -f "$ready_file" ]] || {
  tmux -L "$socket_name" capture-pane -p -t "$target" >&2 || true
  echo "FAIL: fake Agent did not start" >&2
  exit 1
}

baseline=""
previous=""
stable_samples=0
for _ in $(seq 1 200); do
  position="$(tmux -L "$socket_name" display-message -p -t "$target" '#{cursor_x},#{cursor_y}')"
  if [[ "$position" == "$previous" ]]; then
    stable_samples=$((stable_samples + 1))
  else
    stable_samples=0
    previous="$position"
  fi
  if [[ "$stable_samples" -ge 10 ]]; then
    baseline="$position"
    break
  fi
  sleep 0.02
done
[[ -n "$baseline" ]] || {
  echo "FAIL: outer cursor did not stabilize" >&2
  exit 1
}

for _ in $(seq 1 100); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg '[◐◓◑◒]' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" capture-pane -p -t "$target" | rg '[◐◓◑◒]' >/dev/null

touch "$move_file"
while [[ ! -f "$done_file" ]]; do
  position="$(tmux -L "$socket_name" display-message -p -t "$target" '#{cursor_x},#{cursor_y}')"
  if [[ "$position" != "$baseline" ]]; then
    echo "FAIL: outer cursor moved from $baseline to $position" >&2
    exit 1
  fi
  sleep 0.01
done

sleep 0.1
position="$(tmux -L "$socket_name" display-message -p -t "$target" '#{cursor_x},#{cursor_y}')"
[[ "$position" == "$baseline" ]] || {
  echo "FAIL: final outer cursor moved from $baseline to $position" >&2
  exit 1
}

tmux -L "$socket_name" pipe-pane -t "$target"
begin_count="$(LC_ALL=C rg -a -o -F $'\033[?2026h' "$raw_output" | wc -l)"
end_count="$(LC_ALL=C rg -a -o -F $'\033[?2026l' "$raw_output" | wc -l)"
[[ "$begin_count" -gt 0 && "$begin_count" -eq "$end_count" ]] || {
  echo "FAIL: synchronized frame count begin=$begin_count end=$end_count" >&2
  exit 1
}

printf 'PASS: busy hidden cursor kept IME anchor at %s across %s synchronized frames\n' \
  "$baseline" "$begin_count"
