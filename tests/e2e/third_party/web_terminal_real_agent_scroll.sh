#!/usr/bin/env bash
# Drives a real GSDV agent terminal through Chrome and measures TUI scroll updates.
# Example: bash tests/e2e/third_party/web_terminal_real_agent_scroll.sh http://127.0.0.1:9518 http://127.0.0.1:20158 codex
set -euo pipefail

debug_url="${1:?Chrome DevTools HTTP endpoint required}"
page_url="${2:?Served GSDV Web URL required}"
agent_kind="${3:?Agent kind label required}"
if [ "$#" -ge 4 ]; then
  agent_command="$4"
elif [ "$agent_kind" = "codex" ]; then
  agent_command="/model"
else
  agent_command="/help"
fi
scroll_direction="${5:-alternate}"
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
  window.agentScrollProbe = {frames: [], scrolls: [], terminalScroll: false, screenRows: null};
  window.WebSocket = class extends NativeWebSocket {
    constructor(...args) {
      super(...args);
      if (!String(args[0]).includes('/api/agent/output/ws')) return;
      this.addEventListener('message', event => {
        let message;
        try { message = JSON.parse(event.data); } catch (_) { return; }
        if (message.type === 'snapshot') {
          agentScrollProbe.terminalScroll = message.terminal_scroll === true;
          agentScrollProbe.screenRows = message.screen_rows;
        }
        if (message.type === 'snapshot' || message.type === 'patch') {
          agentScrollProbe.frames.push({type: message.type, at: performance.now()});
        }
      });
    }
    send(raw) {
      try {
        const message = JSON.parse(raw);
        if (message.type === 'scroll') {
          agentScrollProbe.scrolls.push({...message, at: performance.now()});
        }
      } catch (_) {}
      return super.send(raw);
    }
  };
})();
JS
)"
cdp Page.enable
cdp Emulation.setDeviceMetricsOverride '{"width":800,"height":420,"deviceScaleFactor":1,"mobile":false}'
cdp Page.addScriptToEvaluateOnNewDocument "$(jq -cn --argjson source "$probe" '{source:$source}')"
cdp Page.navigate "$(jq -cn --arg url "$page_url" '{url:$url}')"
evaluate "window.realAgentScrollCommand=$(jq -Rn --arg value "$agent_command" '$value')"
evaluate "window.realAgentScrollDirection=$(jq -Rn --arg value "$scroll_direction" '$value')"
evaluate '(async()=>{
  for(let i=0;i<200&&!document.querySelector(".remote-shell__menu-button");i++) await new Promise(r=>setTimeout(r,50));
  document.querySelector(".remote-shell__menu-button")?.click();
  for(let i=0;i<200&&!document.querySelector(".remote-shell__drawer-node--root");i++) await new Promise(r=>setTimeout(r,50));
  document.querySelector(".remote-shell__drawer-node--root")?.click();
  for(let i=0;i<100&&!document.querySelector(".remote-shell__agent-node");i++) await new Promise(r=>setTimeout(r,50));
  document.querySelector(".remote-shell__agent-node")?.click();
  for(let i=0;i<400&&agentScrollProbe.frames.length===0;i++) await new Promise(r=>setTimeout(r,50));
  if(agentScrollProbe.frames.length===0) throw new Error(`real agent snapshot did not arrive: ${JSON.stringify({
    agents:document.querySelectorAll(".remote-shell__agent-node").length,
    tabs:document.querySelectorAll(".remote-shell__tab").length,
    banner:document.querySelector(".remote-shell__terminal-banner")?.textContent,
    output:document.querySelector(".remote-shell__terminal-output")?.textContent.slice(0,160)
  })}`);
  if(window.realAgentScrollCommand) {
    const input=document.querySelector(".remote-shell__input");
    input.value=window.realAgentScrollCommand;
    input.dispatchEvent(new InputEvent("input",{bubbles:true,inputType:"insertText",data:window.realAgentScrollCommand}));
    document.querySelector(".remote-shell__send-button")?.click();
  }
  for(let i=0;i<200&&!agentScrollProbe.terminalScroll;i++) await new Promise(r=>setTimeout(r,50));
  if(!agentScrollProbe.terminalScroll) throw new Error("real agent never enabled terminal scroll mode");
  await new Promise(r=>setTimeout(r,2500));
  agentScrollProbe.frames=[];agentScrollProbe.scrolls=[];
  const out=document.querySelector(".remote-shell__terminal-output");
  const rect=out.getBoundingClientRect();
  for(let i=0;i<24;i++) {
    const deltaY=window.realAgentScrollDirection==="up"?-72:
      window.realAgentScrollDirection==="down"?72:i%2===0?72:-72;
    out.dispatchEvent(new WheelEvent("wheel",{bubbles:true,cancelable:true,
      deltaMode:WheelEvent.DOM_DELTA_PIXEL,deltaY,
      clientX:rect.left+rect.width/2,
      clientY:rect.top+Math.min(50,rect.height*0.25)}));
    await new Promise(r=>setTimeout(r,20));
  }
  await new Promise(r=>setTimeout(r,800));
  const patches=agentScrollProbe.frames.filter(frame=>frame.type==="patch");
  const snapshots=agentScrollProbe.frames.filter(frame=>frame.type==="snapshot");
  const intervals=patches.slice(1).map((frame,index)=>frame.at-patches[index].at).sort((a,b)=>a-b);
  const median=intervals.length?intervals[Math.floor(intervals.length/2)]:null;
  const firstPatchMs=patches.length&&agentScrollProbe.scrolls.length?
    patches[0].at-agentScrollProbe.scrolls[0].at:null;
  if(agentScrollProbe.scrolls.length<12) throw new Error(`wheel input stalled: ${agentScrollProbe.scrolls.length}`);
  if(patches.length<1) throw new Error(`scroll produced no patch; screenRows=${agentScrollProbe.screenRows}; scrolls=${JSON.stringify(agentScrollProbe.scrolls.slice(0,3))}; frames=${agentScrollProbe.frames.map(frame=>frame.type)}`);
  if(snapshots.length!==0) throw new Error(`scroll fell back to full snapshots: ${snapshots.length}`);
  if(firstPatchMs===null||firstPatchMs>150) throw new Error(`first scroll feedback is delayed: ${firstPatchMs}ms`);
  if(median!==null&&median>80) throw new Error(`patch cadence is still slideshow-like: ${median.toFixed(1)}ms`);
  return {scrolls:agentScrollProbe.scrolls.length,patches:patches.length,
    snapshots:snapshots.length,firstPatchMs,medianPatchMs:median,
    text:out.textContent.slice(0,120)};
})()'
jq -er --arg kind "$agent_kind" \
  '.result.result.value | {agent:$kind,scrolls,patches,snapshots,firstPatchMs,medianPatchMs,text}' \
  <<< "$cdp_response"
