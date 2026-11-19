#!/usr/bin/env bash
# Verifies that Shift native selection freezes TUI writes while Agent rendering continues.
# Example: bash tests/e2e/third_party/tui_shift_selection_freeze.sh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
target_dir="$(cd "$repo_root" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
binary="${1:-$target_dir/release/gsdv}"
binary="$(realpath "$binary")"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-tui-shift-selection.XXXXXX")"
test_home="$tmp_dir/home"
workspace="$tmp_dir/workspace"
store="$tmp_dir/store.json"
agent_counter="$tmp_dir/agent-counter"
raw_output="$tmp_dir/tui-output.bin"
socket_name="gsdv-tui-shift-selection-$$"
session_name="shift-selection"
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
cat >"$test_home/bin/codex" <<'SH'
#!/bin/sh
index=0
while :; do
  index=$((index + 1))
  printf '%s\n' "$index" >"$E2E_AGENT_COUNTER"
  printf '\033[2J\033[Hagent-frame-%06d\nselection target text %06d' "$index" "$index"
  sleep 0.02
done
SH
chmod +x "$test_home/bin/codex"

jq -n \
  --arg path "$workspace" \
  '{
    active: 0,
    default_agent_kind: "codex",
    runtime_settings: {remote_server_enabled: false},
    workspaces: [{path: $path, agent_kind: "codex", agent_id: "shift-selection-agent"}]
  }' >"$store"

tmux -L "$socket_name" new-session -d -x 120 -y 40 -s "$session_name" \
  "cd '$workspace' && exec env HOME='$test_home' GSDV_STORE_PATH='$store' PATH='$test_home/bin:$PATH' E2E_AGENT_COUNTER='$agent_counter' '$binary' tui --fps 60"
tmux -L "$socket_name" pipe-pane -O -t "$target" "cat >'$raw_output'"

for _ in $(seq 1 300); do
  if [[ -s "$agent_counter" ]] \
    && tmux -L "$socket_name" capture-pane -p -t "$target" | rg -F 'selection target text' >/dev/null; then
    break
  fi
  sleep 0.02
done
[[ -s "$agent_counter" ]] || {
  echo 'FAIL: streaming Agent did not start' >&2
  exit 1
}

# Kitty keyboard protocol: LeftShift press. The terminal emulator normally emits
# this before handling Shift+drag outside application mouse capture.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[57441;1u'
sleep 0.12
frozen_bytes="$(wc -c <"$raw_output")"
counter_before="$(cat "$agent_counter")"
sleep 0.35
frozen_bytes_after="$(wc -c <"$raw_output")"
counter_after="$(cat "$agent_counter")"

[[ "$frozen_bytes_after" -eq "$frozen_bytes" ]] || {
  printf 'FAIL: TUI wrote %s bytes while Shift selection was active\n' \
    "$((frozen_bytes_after - frozen_bytes))" >&2
  exit 1
}
[[ "$counter_after" -gt "$counter_before" ]] || {
  echo 'FAIL: freezing TUI output also stopped the Agent' >&2
  exit 1
}

# Kitty keyboard protocol: LeftShift release. The native selection remains
# visible, so output must stay frozen until the next ordinary application input.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[57441;1:3u'
sleep 0.15
released_bytes="$(wc -c <"$raw_output")"
[[ "$released_bytes" -eq "$frozen_bytes_after" ]] || {
  echo 'FAIL: TUI erased the native selection immediately after Shift release' >&2
  exit 1
}

# An ordinary unshifted click dismisses the terminal selection and lets the TUI
# emit one current frame instead of replaying every intermediate Agent frame.
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;2;2M'
tmux -L "$socket_name" send-keys -t "$target" -l $'\033[<0;2;2m'
for _ in $(seq 1 100); do
  resumed_bytes="$(wc -c <"$raw_output")"
  [[ "$resumed_bytes" -gt "$released_bytes" ]] && break
  sleep 0.02
done
[[ "$resumed_bytes" -gt "$released_bytes" ]] || {
  echo 'FAIL: TUI did not resume drawing after the selection was dismissed' >&2
  exit 1
}

printf 'PASS: Shift selection survives Agent rendering until ordinary input dismisses it\n'
