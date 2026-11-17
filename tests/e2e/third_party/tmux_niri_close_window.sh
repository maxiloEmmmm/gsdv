#!/usr/bin/env bash
set -euo pipefail

# Verifies that Niri closing an Alacritty tmux client detaches the client
# without destroying the session. Example: close-window -> session survives.
socket_name="gsdv-altq-e2e"
session_name="persist"
app_id="GsdvTmuxAltQE2E"

window_id() {
  niri msg --json windows | jq -r --arg app_id "$app_id" \
    '.[] | select(.app_id == $app_id) | .id' | head -n 1
}

cleanup() {
  local id
  id="$(window_id)"
  if [[ -n "$id" ]]; then
    niri msg action close-window --id "$id" >/dev/null 2>&1 || true
  fi
  tmux -L "$socket_name" kill-server >/dev/null 2>&1 || true
}
trap cleanup EXIT

cleanup
tmux -L "$socket_name" new-session -d -s "$session_name"
setsid -f alacritty \
  --title "TMUX-NIRI-CLOSE-E2E" \
  --class "$app_id" \
  -e tmux -L "$socket_name" attach -t "$session_name"

id=""
for _ in $(seq 1 50); do
  id="$(window_id)"
  [[ -n "$id" ]] && break
  sleep 0.1
done
[[ -n "$id" ]] || { echo "FAIL: Alacritty window did not appear" >&2; exit 1; }

client_count="$(tmux -L "$socket_name" list-clients -F '#{client_name}' | wc -l)"
[[ "$client_count" -eq 1 ]] || {
  echo "FAIL: expected one attached client, got $client_count" >&2
  exit 1
}

niri msg action close-window --id "$id" >/dev/null
for _ in $(seq 1 50); do
  [[ -z "$(window_id)" ]] && break
  sleep 0.1
done
[[ -z "$(window_id)" ]] || { echo "FAIL: Niri window still exists" >&2; exit 1; }

tmux -L "$socket_name" has-session -t "$session_name"
client_count="$(tmux -L "$socket_name" list-clients -F '#{client_name}' | wc -l)"
[[ "$client_count" -eq 0 ]] || {
  echo "FAIL: expected no attached clients, got $client_count" >&2
  exit 1
}

echo "PASS: Niri close-window detached Alacritty and tmux session survived"
