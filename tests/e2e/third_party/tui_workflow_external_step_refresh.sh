#!/usr/bin/env bash
# Verifies that externally added workflow steps refresh an already-open TUI Workflow tree.
# Example: bash tests/e2e/third_party/tui_workflow_external_step_refresh.sh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
target_dir="$(cd "$repo_root" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
binary="${1:-$target_dir/release/gsdv}"
binary="$(realpath "$binary")"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-tui-workflow-refresh.XXXXXX")"
test_home="$tmp_dir/home"
workspace="$tmp_dir/workspace"
task="$workspace/gsdv-spec/ps/a111.project/b222.task-01-example.md"
store="$tmp_dir/store.json"
socket_name="gsdv-tui-workflow-refresh-$$"
session_name="workflow-refresh"
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

mkdir -p "$test_home/bin" "$(dirname "$task")"
printf '# Root\n' >"$workspace/gsdv-spec/r001.root.md"
printf '# Project\n' >"$(dirname "$task")/root.md"
printf 'Task body before external steps.\n' >"$task"
printf '%s\n' '#!/bin/sh' 'while :; do sleep 1; done' >"$test_home/bin/codex"
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
      agent_id: "workflow-refresh-agent"
    }]
  }' >"$store"

tmux -L "$socket_name" new-session -d -x 120 -y 40 -s "$session_name" \
  "cd '$workspace' && exec env HOME='$test_home' GSDV_STORE_PATH='$store' PATH='$test_home/bin:$PATH' '$binary' tui --fps 60"

for _ in $(seq 1 300); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '[Workflow]' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '[Workflow]' >/dev/null || {
  echo 'FAIL: TUI main view did not become ready' >&2
  exit 1
}

# Open Workflow, select Project, then select the task while it still has no steps.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;40;40M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;40;40m'
for _ in $(seq 1 300); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'project' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;25;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;25;2m'
for _ in $(seq 1 200); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '01-example' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;50;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;50;2m'
sleep 0.2

pane="$(tmux -L "$socket_name" capture-pane -p -t "$target")"
rg -F 'Task body before external steps.' <<<"$pane" >/dev/null || {
  echo 'FAIL: task did not open before the external write' >&2
  exit 1
}

# Simulate an external Agent/editor adding a step after the TUI tree is already loaded.
printf '%s\n' \
  'Task body before external steps.' \
  '' \
  '## [ ]abc123 NEW_STEP' \
  'Step body.' >"$task"

for _ in $(seq 1 400); do
  pane="$(tmux -L "$socket_name" capture-pane -p -t "$target")"
  if sed -n '2,8p' <<<"$pane" | rg -F 'NEW_STEP' >/dev/null; then
    break
  fi
  sleep 0.025
done
pane="$(tmux -L "$socket_name" capture-pane -p -t "$target")"
sed -n '2,8p' <<<"$pane" | rg -F 'NEW_STEP' >/dev/null || {
  printf 'FAIL: Step column kept the stale workflow tree\n%s\n' "$pane" >&2
  exit 1
}

printf 'PASS: external task changes refresh the open TUI Workflow Step column\n'
