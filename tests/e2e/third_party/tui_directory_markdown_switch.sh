#!/usr/bin/env bash
# Verifies that an asynchronously loaded Dir Markdown replaces the previous file without a second click.
# Example: bash tests/e2e/third_party/tui_directory_markdown_switch.sh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
target_dir="$(cd "$repo_root" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
binary="${1:-$target_dir/release/gsdv}"
binary="$(realpath "$binary")"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-tui-dir-switch.XXXXXX")"
test_home="$tmp_dir/home"
workspace="$tmp_dir/workspace"
store="$tmp_dir/store.json"
socket_name="gsdv-tui-dir-switch-$$"
session_name="dir-switch"
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
printf '# OLD_FILE_MARKER\n' >"$workspace/a.md"
awk 'BEGIN {
  print "# NEW_FILE_MARKER"
  for (row = 0; row < 120000; row++) {
    print "A delayed Markdown paragraph used to cross the first TUI frame."
  }
}' >"$workspace/b.md"

printf '%s\n' \
  '#!/bin/sh' \
  'while :; do sleep 1; done' >"$test_home/bin/codex"
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
      agent_id: "dir-switch-agent"
    }]
  }' >"$store"

tmux -L "$socket_name" new-session -d -x 120 -y 40 -s "$session_name" \
  "cd '$workspace' && exec env HOME='$test_home' GSDV_STORE_PATH='$store' PATH='$test_home/bin:$PATH' '$binary' tui --fps 60"

for _ in $(seq 1 300); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '[Dir]' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '[Dir]' >/dev/null || {
  echo 'FAIL: TUI main view did not become ready' >&2
  exit 1
}

# Open Dir, select workspace root, then load a.md.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;30;40M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;30;40m'
for _ in $(seq 1 200); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'Root' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;1;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;1;2m'
for _ in $(seq 1 200); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'a.md' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;2m'
for _ in $(seq 1 300); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'OLD_FILE_MARKER' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'OLD_FILE_MARKER' >/dev/null || {
  echo 'FAIL: first Markdown did not load' >&2
  exit 1
}

# Select b.md once. Its load finishes after the click frame and must independently dirty the TUI.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;3M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;3m'
for _ in $(seq 1 500); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'NEW_FILE_MARKER' >/dev/null; then
    break
  fi
  sleep 0.02
done
pane="$(tmux -L "$socket_name" capture-pane -p -t "$target")"
rg -F 'NEW_FILE_MARKER' <<<"$pane" >/dev/null || {
  printf 'FAIL: second Markdown required another click\n%s\n' "$pane" >&2
  exit 1
}
if rg -F 'OLD_FILE_MARKER' <<<"$pane" >/dev/null; then
  echo 'FAIL: previous Markdown remained visible after the new load' >&2
  exit 1
fi

printf 'PASS: Dir Markdown switches after one click across asynchronous loading\n'
