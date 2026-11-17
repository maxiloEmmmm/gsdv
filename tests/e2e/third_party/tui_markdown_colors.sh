#!/usr/bin/env bash
# Verifies that TUI Markdown preview uses the shared low-luminance dark palette.
# Example: bash tests/e2e/third_party/tui_markdown_colors.sh target/release/gsdv
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
target_dir="$(cd "$repo_root" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
binary="${1:-$target_dir/release/gsdv}"
binary="$(realpath "$binary")"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-tui-markdown.XXXXXX")"
test_home="$tmp_dir/home"
workspace="$tmp_dir/workspace"
store="$tmp_dir/store.json"
raw_output="$tmp_dir/tui-output.bin"
socket_name="gsdv-tui-markdown-$$"
session_name="markdown"
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
printf '%s\n' \
  '# Palette heading' \
  '' \
  'Body palette fixture.' \
  '' \
  '> Muted palette fixture.' \
  '' \
  '```text' \
  'Code palette fixture.' \
  '```' >"$workspace/README.md"

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
      agent_id: "markdown-fixture-agent"
    }]
  }' >"$store"

tmux -L "$socket_name" new-session -d -x 120 -y 40 -s "$session_name" \
  "cd '$workspace' && exec env HOME='$test_home' GSDV_STORE_PATH='$store' PATH='$test_home/bin:$PATH' '$binary' tui --fps 60"
tmux -L "$socket_name" pipe-pane -O -t "$target" "cat >'$raw_output'"

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

# SGR mouse coordinates are one-based: open Dir, workspace root, then README.md.
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
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'README.md' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;23;2m'

for _ in $(seq 1 300); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'Code palette fixture.' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'Code palette fixture.' >/dev/null || {
  tmux -L "$socket_name" capture-pane -p -t "$target" >&2 || true
  echo 'FAIL: Markdown preview did not load' >&2
  exit 1
}

LC_ALL=C rg -a -F $'\033[0;1;38;2;96;165;250;48;2;11;16;22mPalette heading' "$raw_output" >/dev/null
LC_ALL=C rg -a -F $'\033[0;38;2;163;174;187;48;2;11;16;22mBody palette fixture.' "$raw_output" >/dev/null
LC_ALL=C rg -a -F $'\033[0;38;2;145;164;183;48;2;11;16;22m│ Muted palette fixture.' "$raw_output" >/dev/null
LC_ALL=C rg -a -F $'\033[0;38;2;147;172;202;48;2;11;16;22mCode palette fixture.' "$raw_output" >/dev/null

if LC_ALL=C rg -a -F $'38;2;226;232;240;48;2;11;16;22mBody palette fixture.' "$raw_output" >/dev/null; then
  echo 'FAIL: Markdown body still uses the old bright color' >&2
  exit 1
fi
if LC_ALL=C rg -a -F $'38;2;45;212;191;48;2;11;16;22mCode palette fixture.' "$raw_output" >/dev/null; then
  echo 'FAIL: Markdown code still uses the old saturated color' >&2
  exit 1
fi

# The workbench starts at column 45; click its Edit button and verify raw source styling.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;55;1M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;55;1m'
for _ in $(seq 1 200); do
  if tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '# Palette heading' >/dev/null; then
    break
  fi
  sleep 0.02
done
tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F '# Palette heading' >/dev/null || {
  echo 'FAIL: Markdown editor did not open' >&2
  exit 1
}
tmux -L "$socket_name" pipe-pane -t "$target"
LC_ALL=C rg -a -F $'\033[0;38;2;163;174;187;48;2;11;16;22m# Palette heading' "$raw_output" >/dev/null

printf 'PASS: TUI Markdown preview and editor use the shared dark palette\n'
