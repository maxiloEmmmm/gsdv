#!/usr/bin/env bash
# Measures native session-history scrolling and reconnect through Chrome CDP.
# Example: bash tests/e2e/third_party/web_terminal_real_agent_scroll.sh http://127.0.0.1:9518 http://127.0.0.1:20051 codex
set -euo pipefail

debug_url="${1:?Chrome DevTools HTTP endpoint required}"
page_url="${2:?Served GSDV Web URL required}"
agent_kind="${3:?Agent kind label required}"
screenshot_path="${4:-}"
viewport_width="${5:-800}"
viewport_height="${6:-420}"
target="$(curl --fail --silent --show-error -X PUT "$debug_url/json/new?about:blank")"
target_id="$(jq -er .id <<< "$target")"
target_ws="$(jq -er .webSocketDebuggerUrl <<< "$target")"
coproc AGENT_CDP { websocat -t -B 4194304 "$target_ws"; }
cdp_pid="$AGENT_CDP_PID"
exec {cdp_read}<&"${AGENT_CDP[0]}"
exec {cdp_write}>&"${AGENT_CDP[1]}"
request_id=0
cdp_response='{}'

# Releases this check's browser page and CDP connection.
cleanup() {
  curl --silent --max-time 2 "$debug_url/json/close/$target_id" >/dev/null || true
  kill "$cdp_pid" 2>/dev/null || true
  wait "$cdp_pid" 2>/dev/null || true
}
trap cleanup EXIT

# Waits for one matching CDP reply while skipping asynchronous events.
cdp() {
  request_id=$((request_id + 1))
  local request response params="${2-}"
  if [ -z "$params" ]; then params='{}'; fi
  request="$(jq -cn --argjson id "$request_id" --arg method "$1" \
    --argjson params "$params" '{id:$id,method:$method,params:$params}')"
  printf '%s\n' "$request" >&"$cdp_write"
  while IFS= read -r -t 30 response <&"$cdp_read"; do
    if jq -e --argjson id "$request_id" '.id == $id' <<< "$response" >/dev/null; then
      cdp_response="$response"
      if jq -e 'has("error") or (.result | has("exceptionDetails"))' <<< "$response" >/dev/null; then
        printf '%s\n' "$response" >&2
        return 1
      fi
      return 0
    fi
  done
  printf 'CDP request timed out: %s\n' "$1" >&2
  return 1
}

# Evaluates an async browser expression and keeps its JSON result.
evaluate() {
  cdp Runtime.evaluate "$(jq -cn --arg expression "$1" \
    '{expression:$expression,returnByValue:true,awaitPromise:true}')"
}

probe="$(jq -Rs . <<'JS'
(() => {
  const NativeWebSocket = window.WebSocket;
  window.sessionProbe = {urls: [], sent: [], socket: null};
  window.WebSocket = class extends NativeWebSocket {
    constructor(...args) {
      super(...args);
      if (!String(args[0]).includes('/api/agent/session/ws')) return;
      sessionProbe.urls.push(String(args[0]));
      sessionProbe.socket = this;
    }
    send(raw) {
      if (this === sessionProbe.socket) sessionProbe.sent.push(String(raw));
      return super.send(raw);
    }
  };
})();
JS
)"
cdp Page.enable
cdp Emulation.setDeviceMetricsOverride "$(jq -cn --argjson width "$viewport_width" \
  --argjson height "$viewport_height" \
  '{width:$width,height:$height,deviceScaleFactor:1,mobile:false}')"
cdp Page.addScriptToEvaluateOnNewDocument "$(jq -cn --argjson source "$probe" '{source:$source}')"
cdp Page.navigate "$(jq -cn --arg url "$page_url" '{url:$url}')"
evaluate '(async()=>{
  for(let i=0;i<200&&!document.querySelector(".remote-shell__menu-button");i++) await new Promise(r=>setTimeout(r,50));
  document.querySelector(".remote-shell__menu-button")?.click();
  for(let i=0;i<200&&!document.querySelector(".remote-shell__drawer-node--root");i++) await new Promise(r=>setTimeout(r,50));
  document.querySelector(".remote-shell__drawer-node--root")?.click();
  for(let i=0;i<100&&!document.querySelector(".remote-shell__agent-node");i++) await new Promise(r=>setTimeout(r,50));
  document.querySelector(".remote-shell__agent-node")?.click();
  for(let i=0;i<600&&document.querySelectorAll(".remote-shell__session-item,.remote-shell__session-tool").length<20;i++) await new Promise(r=>setTimeout(r,50));
  const out=document.querySelector(".remote-shell__terminal-output");
  const count=document.querySelectorAll(".remote-shell__session-item,.remote-shell__session-tool").length;
  if(count<20) throw new Error(`session did not render: count=${count}, banner=${document.querySelector(".remote-shell__terminal-banner")?.textContent}, text=${out?.textContent.slice(0,120)}`);
  if(out.scrollHeight<=out.clientHeight*2) throw new Error(`session history is not scrollable: ${out.scrollHeight}/${out.clientHeight}`);
  let stableHeight=0,stableFrames=0;
  for(let i=0;i<200&&stableFrames<8;i++) {
    await new Promise(r=>setTimeout(r,25));
    if(out.scrollHeight===stableHeight) stableFrames+=1; else {stableHeight=out.scrollHeight;stableFrames=0;}
  }
  const bottomDistance=out.scrollHeight-out.clientHeight-out.scrollTop;
  if(bottomDistance>2) throw new Error(`initial session did not follow bottom: ${bottomDistance}px`);
  const emptyMessages=[...document.querySelectorAll(".remote-shell__session-item")]
    .filter(node=>!node.textContent.trim()).length;
  if(emptyMessages!==0) throw new Error(`empty session messages rendered: ${emptyMessages}`);
  const links=[...document.querySelectorAll(".remote-shell__session-markdown a")];
  if(links.some(link=>link.target!=="_blank"||!link.relList.contains("noopener")||!link.relList.contains("noreferrer"))) throw new Error("Markdown link did not open safely in a new tab");
  return {count,scrollTop:out.scrollTop,scrollHeight:out.scrollHeight,clientHeight:out.clientHeight,bottomDistance,emptyMessages,links:links.length};
})()'
initial="$(jq -c '.result.result.value' <<< "$cdp_response")"
if [ -n "$screenshot_path" ]; then
  evaluate 'new Promise(resolve=>setTimeout(()=>resolve(true),500))'
  cdp Page.captureScreenshot '{"format":"png","captureBeyondViewport":false}'
  jq -er '.result.data' <<< "$cdp_response" | base64 -d >"$screenshot_path"
fi

evaluate 'sessionProbe.wheelBefore=document.querySelector(".remote-shell__terminal-output").scrollTop'
cdp Input.synthesizeScrollGesture '{"x":500,"y":200,"yDistance":480,"speed":2400,"gestureSourceType":"mouse"}'
evaluate '(async()=>{
  const out=document.querySelector(".remote-shell__terminal-output");
  const started=performance.now();
  const before=sessionProbe.wheelBefore;
  for(let i=0;i<10&&out.scrollTop===before;i++) await new Promise(requestAnimationFrame);
  const responseMs=performance.now()-started;
  const sent=sessionProbe.sent.map(raw=>{try{return JSON.parse(raw).type}catch(_){return "invalid"}});
  if(!(out.scrollTop<before)) throw new Error(`native wheel did not move: ${before} -> ${out.scrollTop}`);
  if(sent.includes("scroll")||sent.includes("resize")) throw new Error(`wheel leaked to PTY protocol: ${sent}`);
  if(responseMs>100) throw new Error(`native wheel response is delayed: ${responseMs}ms`);
  return {before,after:out.scrollTop,responseMs,sent};
})()'
wheel="$(jq -c '.result.result.value' <<< "$cdp_response")"

evaluate '(async()=>{
  const out=document.querySelector(".remote-shell__terminal-output");
  const countBefore=document.querySelectorAll(".remote-shell__session-item,.remote-shell__session-tool").length;
  const socket=sessionProbe.socket;
  socket.close();
  for(let i=0;i<200&&sessionProbe.urls.length<2;i++) await new Promise(r=>setTimeout(r,25));
  if(sessionProbe.urls.length<2) throw new Error("session socket did not reconnect");
  const reconnect=new URL(sessionProbe.urls.at(-1));
  const seek=Number(reconnect.searchParams.get("seek"));
  if(!(seek>0)) throw new Error(`reconnect did not resume seek: ${reconnect}`);
  for(let i=0;i<100&&sessionProbe.socket.readyState!==WebSocket.OPEN;i++) await new Promise(r=>setTimeout(r,25));
  const countAfter=document.querySelectorAll(".remote-shell__session-item,.remote-shell__session-tool").length;
  if(countAfter!==countBefore) throw new Error(`same-session reconnect cleared output: ${countBefore} -> ${countAfter}`);
  if(out.scrollTop<0) throw new Error("invalid scroll position after reconnect");
  return {connections:sessionProbe.urls.length,seek,countBefore,countAfter};
})()'
reconnect="$(jq -c '.result.result.value' <<< "$cdp_response")"

jq -cn --arg agent "$agent_kind" --argjson initial "$initial" --argjson wheel "$wheel" \
  --argjson reconnect "$reconnect" '{agent:$agent,initial:$initial,wheel:$wheel,reconnect:$reconnect}'
