#!/usr/bin/env bash
# Verifies UTF-8 Markdown cursor movement with all four arrow keys in the real TUI.
# Example: bash tests/e2e/third_party/tui_markdown_arrow_keys.sh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
target_dir="$(cd "$repo_root" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
binary="${1:-$target_dir/release/gsdv}"
binary="$(realpath "$binary")"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-tui-markdown-arrows.XXXXXX")"
test_home="$tmp_dir/home"
workspace="$tmp_dir/workspace"
document="$workspace/README.md"
store="$tmp_dir/store.json"
socket_name="gsdv-tui-markdown-arrows-$$"
session_name="markdown-arrows"
target="$session_name:0.0"

# Stops only this test's isolated tmux server and process tree.
cleanup() {
  pane_pid="$(tmux -L "$socket_name" list-panes -t "$target" -F '#{pane_pid}' 2>/dev/null | head -n 1 || true)"
  if [[ -n "$pane_pid" ]]; then
    pkill -TERM -P "$pane_pid" 2>/dev/null || true
    kill "$pane_pid" 2>/dev/null || true
  fi
  tmux -L "$socket_name" kill-server >/dev/null 2>&1 || true
  find "$tmp_dir" -depth -delete
}
trap cleanup EXIT

mkdir -p "$test_home/bin" "$workspace"
printf '%s\n%s\n%s' \
  '0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789' \
  '短' \
  'ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUV' \
  >"$document"
printf '%s\n' '#!/bin/sh' 'while :; do sleep 1; done' >"$test_home/bin/codex"
chmod +x "$test_home/bin/codex"

jq -n \
  --arg path "$workspace" \
  '{
    active: 0,
    default_agent_kind: "codex",
    runtime_settings: {remote_server_enabled: false},
    workspaces: [{path: $path, agent_kind: "codex", agent_id: "markdown-arrows-agent"}]
  }' >"$store"

tmux -L "$socket_name" new-session -d -x 120 -y 40 -s "$session_name" \
  "cd '$workspace' && exec env HOME='$test_home' GSDV_STORE_PATH='$store' PATH='$test_home/bin:$PATH' '$binary' tui --fps 60"

for _ in $(seq 1 300); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '[Dir]' >/dev/null; then
    break
  fi
  sleep 0.02
done

# Open Dir, select workspace root and README, then enter Edit mode.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;30;40M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;30;40m'
for _ in $(seq 1 200); do
  tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'Root' >/dev/null && break
  sleep 0.02
done
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;1;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;1;2m'
for _ in $(seq 1 200); do
  tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'README.md' >/dev/null && break
  sleep 0.02
done
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;2m'
for _ in $(seq 1 300); do
  tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '0123456789' >/dev/null && break
  sleep 0.02
done
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;55;1M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;55;1m'
for _ in $(seq 1 200); do
  tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '[Edit]' >/dev/null && break
  sleep 0.02
done

# Cursor starts at EOF. Up preserves column across a short UTF-8 line; Left/Right
# move by characters. Insertions make every movement observable in the saved file.
tmux -L "$socket_name" send-keys -t "$target" Up Up Left Left Right
tmux -L "$socket_name" send-keys -t "$target" -l 'X'
tmux -L "$socket_name" send-keys -t "$target" Down
tmux -L "$socket_name" send-keys -t "$target" -l 'Y'
tmux -L "$socket_name" send-keys -t "$target" Down
tmux -L "$socket_name" send-keys -t "$target" -l 'Z'

# Save via the workbench toolbar.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;64;1M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;64;1m'
for _ in $(seq 1 300); do
  if ! tmux -L "$socket_name" capture-pane -p -t "$target" | rg '\*$' >/dev/null \
    && rg -F '短Y' "$document" >/dev/null; then
    break
  fi
  sleep 0.02
done

mapfile -t lines <"$document"
[[ "${lines[0]}" == '012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678X9' ]] || {
  printf 'FAIL: Up/Left/Right produced the wrong first line\n%s\n' "${lines[0]}" >&2
  exit 1
}
[[ "${lines[1]}" == '短Y' ]] || {
  printf 'FAIL: Down did not land at the UTF-8 short-line end\n%s\n' "${lines[1]}" >&2
  exit 1
}
[[ "${lines[2]}" == 'ABCZDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUV' ]] || {
  printf 'FAIL: the second Down did not preserve the new display column\n%s\n' "${lines[2]}" >&2
  exit 1
}

printf 'PASS: TUI Markdown editor handles Up/Down/Left/Right across UTF-8 and long lines\n'
