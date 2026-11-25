#!/usr/bin/env bash
# Exercises TUI provider selection and verifies Claude's precedence against a loopback gateway.
# Example: bash tests/e2e/third_party/claude_provider.sh
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../../.." && pwd)"
target_dir="$(cd "$repo_root" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"
binary="$(realpath "${1:-$target_dir/release/gsdv}")"
claude="$(command -v claude)"
fixture="$(mktemp -d)"
test_home="$fixture/home"
workspace="$fixture/workspace"
store="$fixture/store.json"
profile="$test_home/.gsdv/claudes/gateway.json"
socket="gsdv-claude-provider-$$"
target="provider:0.0"
server_pid=

# Stops isolated processes and removes their temporary fixture on exit.
cleanup() {
  tmux -L "$socket" kill-server 2>/dev/null || true
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  find "$fixture" -depth -delete
}
trap cleanup EXIT

# Waits for an observable result. Example: wait_for pane_has '[Dir]'.
wait_for() {
  for ((attempt=0; attempt<160; attempt++)); do
    if "$@"; then return; fi
    sleep 0.03
  done
  printf 'Timed out waiting for %s\n' "$*" >&2
  tmux -L "$socket" capture-pane -p -t "$target" >&2 || true
  return 1
}

# Tests rendered TUI text. Example: pane_has gateway -> status 0.
pane_has() {
  tmux -L "$socket" capture-pane -p -t "$target" | rg -F -- "$1" >/dev/null
}

# Emits a click at one-based coordinates. Example: click 2 31 1 -> right click tab.
click() {
  local sequence
  printf -v sequence '\033[<%s;%s;%sM\033[<%s;%s;%sm' "$1" "$2" "$3" "$1" "$2" "$3"
  tmux -L "$socket" send-keys -t "$target" -l "$sequence"
}

# Opens the model provider submenu. Example: on the Agent tab -> None and gateway.
open_providers() {
  click 2 31 1
  wait_for pane_has 'Set model provider'
  local row
  row="$(tmux -L "$socket" capture-pane -p -t "$target" |
    awk 'index($0,"Set model provider") {print NR; exit}')"
  click 0 33 "$row"
  wait_for pane_has 'gateway'
}

mkdir -p "$test_home/bin" "$test_home/.claude" "$(dirname "$profile")" "$workspace"
jq -n --arg path "$workspace" '{
  active: 0, default_agent_kind: "claude", runtime_settings: {remote_server_enabled: false},
  workspaces: [{path: $path, agent_kind: "claude", agent_id: "claude-provider-fixture"}]
}' >"$store"
printf '%s\n' '{"env":{}}' >"$profile"
printf '%s\n' '#!/bin/sh' 'printf "%s\n" "$@" --end-- >>"$PROVIDER_ARG_LOG"' \
  'printf "Claude fixture ready\n"' 'while :; do sleep 1; done' >"$test_home/bin/claude"
chmod +x "$test_home/bin/claude"

tmux -L "$socket" new-session -d -x 120 -y 40 -s provider \
  "cd '$workspace' && exec env HOME='$test_home' GSDV_STORE_PATH='$store' PATH='$test_home/bin:$PATH' PROVIDER_ARG_LOG='$fixture/args' '$binary' tui --fps 60"
wait_for pane_has 'Claude fixture ready'
open_providers
tmux -L "$socket" send-keys -t "$target" Down Enter
wait_for rg -qFx -- "$profile" "$fixture/args"
rg -qFx -- '--settings' "$fixture/args"
wait_for jq -e '.workspaces[0].agent_model_provider == "gateway"' "$store" >/dev/null
open_providers
tmux -L "$socket" send-keys -t "$target" Enter
wait_for jq -e '.workspaces[0].agent_model_provider == null' "$store" >/dev/null
wait_for awk '/^--end--$/ { count++ } END { exit(count < 3) }' "$fixture/args"
awk '/^--settings$/ { count++ } END { exit(count != 1) }' "$fixture/args"
tmux -L "$socket" kill-server

perl -MIO::Socket::INET -e '
    $| = 1;
    my $server = IO::Socket::INET->new(LocalAddr => "127.0.0.1", LocalPort => 0, Listen => 8, ReuseAddr => 1) or die $!;
    print $server->sockport, "\n";
    while (my $client = $server->accept) {
        $client->autoflush(1);
        my $request = <$client> // "";
        my ($auth, $marker) = ("", "");
        while (my $header = <$client>) {
            last if $header =~ /^\r?$/;
            $auth = $1 if $header =~ /^Authorization:\s*(.*?)\r?$/i;
            $marker = $1 if $header =~ /^X-Gsdv-Probe:\s*(.*?)\r?$/i;
        }
        if ($request =~ m{^POST (\S*/v1/messages)(?:[?\s])}) {
            print "$1|$auth|$marker\n";
        }
        my $body = q({"type":"error","error":{"type":"invalid_request_error","message":"local fixture"}});
        print $client "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: ", length($body), "\r\nConnection: close\r\n\r\n", $body;
        close $client;
    }
' >"$fixture/requests" &
server_pid=$!
wait_for test -s "$fixture/requests"
base_url="http://127.0.0.1:$(head -n 1 "$fixture/requests")"
jq -n --arg url "$base_url/user" '{env: {
  ANTHROPIC_BASE_URL: $url, ANTHROPIC_AUTH_TOKEN: "user-key",
  ANTHROPIC_CUSTOM_HEADERS: "X-Gsdv-Probe: user-env"
}}' >"$test_home/.claude/settings.json"
jq -n --arg url "$base_url/selected" '{env: {
  ANTHROPIC_BASE_URL: $url, ANTHROPIC_AUTH_TOKEN: "selected-key"
}}' >"$profile"

# Probes the real CLI with synthetic credentials. Example: --settings PATH -> selected gateway.
probe_claude() (
  cd "$workspace"
  timeout 20 env -i HOME="$test_home" PATH="$PATH" TERM=xterm \
    DISABLE_TELEMETRY=1 CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
    ANTHROPIC_BASE_URL="$base_url/shell" ANTHROPIC_AUTH_TOKEN=shell-key \
    "$claude" --setting-sources user --no-session-persistence --print \
    --model claude-sonnet-4-6 "$@" 'Reply with hello.' >"$fixture/claude.log" 2>&1 || true
)

probe_claude --settings "$profile"
rg -qFx '/selected/v1/messages|Bearer selected-key|user-env' "$fixture/requests" || {
  sed -n '1,10p' "$fixture/claude.log" >&2
  exit 1
}
probe_claude
rg -qFx '/user/v1/messages|Bearer user-key|user-env' "$fixture/requests" || {
  sed -n '1,10p' "$fixture/claude.log" >&2
  exit 1
}
printf 'PASS: TUI profile selection and Claude request precedence\n'
