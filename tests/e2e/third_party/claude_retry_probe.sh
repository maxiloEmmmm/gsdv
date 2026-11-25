#!/usr/bin/env bash
# Exercises retry backoff and budget against a loopback-only gateway.
# Example: bash tests/e2e/third_party/claude_retry_probe.sh [path-for-verified-copy]
set -euo pipefail

patcher="$(cd "$(dirname "$0")/../../.." && pwd)/scripts/claude-retry-patch"
source_binary="$(readlink -f "${CLAUDE_RETRY_SOURCE:-$(command -v claude)}")"
if [[ -z "${CLAUDE_RETRY_SOURCE:-}" && -f "$source_binary.retry-original" ]]; then
  source_binary="$source_binary.retry-original"
fi
fixture="$(mktemp -d)"
probe_binary="$fixture/claude-target"
output_path="${1:-}"
server_pid=

# Cleans up only the test gateway and executable copy.
cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  find "$fixture" -depth -delete
}
trap cleanup EXIT

mkdir "$fixture/home" "$fixture/workspace"
cp -- "$source_binary" "$probe_binary"
chmod 700 "$probe_binary"
ln -s -- "$probe_binary" "$fixture/claude"
"$patcher" "$fixture/claude"
"$patcher" "$fixture/claude" | rg -q '^Already patched:'
[[ -L "$fixture/claude" ]]
cmp -s -- "$source_binary" "$probe_binary.retry-original"
[[ ! -e "$probe_binary.retry-original.1" ]]

perl -MIO::Socket::INET -MTime::HiRes=time -e '
    $|=1;
    my $server=IO::Socket::INET->new(LocalAddr=>"127.0.0.1",LocalPort=>0,Listen=>8) or die $!;
    print $server->sockport,"\n";
    while (my $client=$server->accept) {
        my $request=<$client>//"";
        my $body_length=0;
        while (my $header=<$client>) {
            last if $header =~ /^\r?$/;
            $body_length=$1 if $header =~ /^Content-Length: (\d+)/i;
        }
        read($client,my $request_body,$body_length) if $body_length;
        my ($code)=$request =~ m{^POST /(\d+)/v1/messages(?:[?\s])};
        $code //= 400;
        printf "%d %.3f\n",$code,time() if $code != 400;
        my $type=$code==529 ? "overloaded_error" : $code==429 ? "rate_limit_error" : "api_error";
        my $retry_after=$code==429 ? "Retry-After: 3\r\n" : "";
        my $body=qq({"type":"error","error":{"type":"$type","message":"loopback fixture"}});
        print $client "HTTP/1.1 $code Fixture\r\n",$retry_after,"Content-Type: application/json\r\nContent-Length: ",length($body),"\r\nConnection: close\r\n\r\n",$body;
        close $client;
    }
' >"$fixture/requests" &
server_pid=$!
for ((attempt=0; attempt<80; attempt++)); do
  [[ -s "$fixture/requests" ]] && break
  sleep 0.025
done
base_url="http://127.0.0.1:$(head -n 1 "$fixture/requests")"
cd "$fixture/workspace"

# Runs a disposable Claude session. Example: probe 502 -> loopback HTTP 502.
probe() {
  local status=0
  timeout 7 env -i HOME="$fixture/home" PATH="$PATH" TERM=xterm \
    DISABLE_TELEMETRY=1 CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1 \
    CLAUDE_CODE_RETRY_WATCHDOG=1 CLAUDE_CODE_MAX_RETRIES=1 \
    CLAUDE_CODE_RETRY_WATCHDOG_MAX_WAIT_MS=1000 \
    ANTHROPIC_BASE_URL="$base_url/$1" ANTHROPIC_AUTH_TOKEN=fixture-key \
    "$probe_binary" --setting-sources "" --strict-mcp-config --no-session-persistence \
    --model claude-sonnet-4-6 --print 'Reply with hello.' \
    >"$fixture/result" 2>"$fixture/stderr" || status=$?
  [[ "$status" == 124 ]] || { tail -n 5 "$fixture/result" "$fixture/stderr" "$fixture/requests" >&2; return 1; }
}

probe 502
probe 529
probe 429
awk '
    $1 ~ /^(429|502|529)$/ {
        if (count[$1]++) {
            delta=$2-last[$1]
            if (delta>max[$1]) max[$1]=delta
            if (!min[$1] || delta<min[$1]) min[$1]=delta
        }
        last[$1]=$2
    }
    END {
        for (status in count) printf "%s attempts=%d interval=%.3f–%.3fs\n",status,count[status],min[status],max[status]
        exit(count[502]<4 || count[529]<4 || max[502]>2.8 || max[529]>2.8 || count[429]<2 || min[429]<2.9)
    }
' "$fixture/requests"

if [[ -n "$output_path" ]]; then
  cp --no-clobber --preserve=mode -- "$probe_binary" "$output_path"
  cmp -s -- "$probe_binary" "$output_path"
fi
