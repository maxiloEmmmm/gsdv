#!/usr/bin/env bash
# Verifies that every TUI Workflow column sorts by visible names, not stable IDs.
# Example: bash tests/e2e/third_party/tui_workflow_column_sort.sh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
target_dir="$(cd "$repo_root" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
binary="${1:-$target_dir/release/gsdv}"
binary="$(realpath "$binary")"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-tui-workflow-sort.XXXXXX")"
test_home="$tmp_dir/home"
workspace="$tmp_dir/workspace"
store="$tmp_dir/store.json"
raw_output="$tmp_dir/tui-output.bin"
socket_name="gsdv-tui-workflow-sort-$$"
session_name="workflow-sort"
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

mkdir -p "$test_home/bin" "$workspace/gsdv-spec/ps/a111.zulu" \
  "$workspace/gsdv-spec/ps/b222.alpha" "$workspace/gsdv-spec/ps/c333.middle" \
  "$workspace/a-repo/.git" "$workspace/a-repo/gsdv-spec/ps" \
  "$workspace/z-repo/.git" "$workspace/z-repo/gsdv-spec/ps"

printf '# Root\n' >"$workspace/gsdv-spec/r001.root.md"
printf '# Alpha\n' >"$workspace/gsdv-spec/ps/b222.alpha/root.md"
printf '# Middle\n' >"$workspace/gsdv-spec/ps/c333.middle/root.md"
printf '# Zulu\n' >"$workspace/gsdv-spec/ps/a111.zulu/root.md"
printf '# A repo\n' >"$workspace/a-repo/gsdv-spec/r002.root.md"
printf '# Z repo\n' >"$workspace/z-repo/gsdv-spec/r003.root.md"

printf '%s\n' \
  '# Task 10' \
  '' \
  '## [ ]aaaaaa 30-zulu' \
  'zulu body' \
  '' \
  '## [ ]bbbbbb 10-alpha' \
  'alpha body' \
  '' \
  '## [ ]cccccc 20-middle' \
  'middle body' >"$workspace/gsdv-spec/ps/b222.alpha/b222.task-10-alpha.md"
printf '# Task 20\n' >"$workspace/gsdv-spec/ps/b222.alpha/c333.task-20-middle.md"
printf '# Task 30\n' >"$workspace/gsdv-spec/ps/b222.alpha/a111.task-30-zulu.md"

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
      agent_id: "workflow-sort-agent"
    }]
  }' >"$store"

tmux -L "$socket_name" new-session -d -x 120 -y 40 -s "$session_name" \
  "cd '$workspace' && exec env HOME='$test_home' GSDV_STORE_PATH='$store' PATH='$test_home/bin:$PATH' '$binary' tui --fps 60"
tmux -L "$socket_name" pipe-pane -O -t "$target" "cat >'$raw_output'"

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

# Open Workflow, then select the first visible Project and Task rows.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;36;40M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;36;40m'
for _ in $(seq 1 300); do
  pane="$(tmux -L "$socket_name" capture-pane -p -t "$target")"
  if rg -F 'a-repo' <<<"$pane" >/dev/null && rg -F 'alpha' <<<"$pane" >/dev/null; then
    break
  fi
  sleep 0.02
done

tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;2m'
for _ in $(seq 1 200); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '10-alpha' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;45;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;45;2m'

for _ in $(seq 1 200); do
  pane="$(tmux -L "$socket_name" capture-pane -p -t "$target")"
  if sed -n '2p' <<<"$pane" | rg '10-alpha.*10-alpha' >/dev/null; then
    break
  fi
  sleep 0.02
done
pane="$(tmux -L "$socket_name" capture-pane -p -t "$target")"
line_one="$(sed -n '2p' <<<"$pane")"
line_two="$(sed -n '3p' <<<"$pane")"
line_three="$(sed -n '4p' <<<"$pane")"

rg 'Root.*alpha.*10-alpha.*10-alpha' <<<"$line_one" >/dev/null || {
  printf 'FAIL: first rows are not sorted: %s\n' "$line_one" >&2
  exit 1
}
rg 'a-repo.*middle.*20-middle.*20-middle' <<<"$line_two" >/dev/null || {
  printf 'FAIL: second rows are not sorted: %s\n' "$line_two" >&2
  exit 1
}
rg 'z-repo.*zulu.*30-zulu.*30-zulu' <<<"$line_three" >/dev/null || {
  printf 'FAIL: third rows are not sorted: %s\n' "$line_three" >&2
  exit 1
}

tmux -L "$socket_name" pipe-pane -t "$target"
LC_ALL=C rg -a -F '48;2;30;64;175mRoot' "$raw_output" >/dev/null || {
  echo 'FAIL: selected Repo row is not visibly highlighted' >&2
  exit 1
}
LC_ALL=C rg -a -F '48;2;30;64;175malpha' "$raw_output" >/dev/null || {
  echo 'FAIL: selected Project row is not visibly highlighted' >&2
  exit 1
}
LC_ALL=C rg -a -F '48;2;30;64;175m10-alpha' "$raw_output" >/dev/null || {
  echo 'FAIL: selected Task row is not visibly highlighted' >&2
  exit 1
}

printf 'PASS: Workflow columns sort by visible names and highlight the current path\n'
