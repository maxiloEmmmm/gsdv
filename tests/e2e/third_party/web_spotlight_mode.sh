#!/usr/bin/env bash
# Drives the served Web bundle through an existing headless Chrome CDP endpoint.
# Example: bash tests/e2e/third_party/web_spotlight_mode.sh http://127.0.0.1:9448 http://127.0.0.1:4184
set -euo pipefail

debug_url="${1:?Chrome DevTools HTTP endpoint required}"
page_url="${2:?Served GSDV Web URL required}"
screenshot_path="${3-}"
target="$(curl --fail --silent --show-error -X PUT "$debug_url/json/new?about:blank")"
target_id="$(jq -er .id <<<"$target")"
target_ws="$(jq -er .webSocketDebuggerUrl <<<"$target")"
coproc SPOTLIGHT_CDP { websocat -t -B 4194304 "$target_ws"; }
cdp_pid="$SPOTLIGHT_CDP_PID"
exec {cdp_read}<&"${SPOTLIGHT_CDP[0]}"
exec {cdp_write}>&"${SPOTLIGHT_CDP[1]}"
request_id=0
cdp_response='{}'

# Releases only this check's page and CDP connection.
cleanup() {
  curl --silent --max-time 2 "$debug_url/json/close/$target_id" >/dev/null || true
  kill "$cdp_pid" 2>/dev/null || true
  wait "$cdp_pid" 2>/dev/null || true
}
trap cleanup EXIT

# Sends one CDP command and waits for its matching reply.
cdp() {
  request_id=$((request_id + 1))
  local request response params="${2-}"
  if [ -z "$params" ]; then params='{}'; fi
  request="$(jq -cn --argjson id "$request_id" --arg method "$1" \
    --argjson params "$params" '{id:$id,method:$method,params:$params}')"
  printf '%s\n' "$request" >&"$cdp_write"
  while IFS= read -r -t 10 response <&"$cdp_read"; do
    if jq -e --argjson id "$request_id" '.id == $id' <<<"$response" >/dev/null; then
      cdp_response="$response"
      if jq -e 'has("error") or (.result | has("exceptionDetails"))' <<<"$response" >/dev/null; then
        printf '%s\n' "$response" >&2
        return 1
      fi
      return 0
    fi
  done
  printf 'CDP request timed out: %s\n' "$1" >&2
  return 1
}

# Evaluates an asynchronous browser assertion.
evaluate() {
  cdp Runtime.evaluate "$(jq -cn --arg expression "$1" \
    '{expression:$expression,returnByValue:true,awaitPromise:true}')"
}

fixture="$(jq -Rs . <<'JS'
(() => {
  const agent = (id,title,activity,inSpotlight=true) => ({
    agent_id:id,title,activity,in_spotlight:inSpotlight,manually_stopped:false
  });
  window.spotlightFixture = {
    workspaceConnections:0,
    workspaceSocket:null,
    agentConnections:0,
    agentMessages:[],
    workspaces:[{
      workspace_id:"workspace-a",name:"alpha",path:"/alpha",rows:[{
        row_index:0,cols:[{col_index:0,agents:[
          agent("busy-agent","Busy Agent","busy"),
          agent("idle-agent","Idle Agent","idle"),
          agent("plain-agent","Plain Agent","unknown",false)
        ]}]
      }]
    }],
    emitWorkspace() {
      this.workspaceSocket?.onmessage?.({data:JSON.stringify({workspaces:this.workspaces})});
    },
    swapActivity() {
      const agents=this.workspaces[0].rows[0].cols[0].agents;
      agents[0].activity="idle";
      agents[1].activity="busy";
      this.emitWorkspace();
    }
  };
  window.fetch = async () => new Response(JSON.stringify({
    workspaces:spotlightFixture.workspaces
  }));
  window.WebSocket = class {
    static OPEN=1;
    readyState=1;
    constructor(url) {
      this.url=String(url);
      this.workspace=this.url.includes("/api/workspace/ws");
      if(this.workspace) {
        spotlightFixture.workspaceConnections+=1;
        spotlightFixture.workspaceSocket=this;
        setTimeout(()=>{this.onopen?.();spotlightFixture.emitWorkspace();},20);
      } else {
        spotlightFixture.agentConnections+=1;
        setTimeout(()=>{
          this.onopen?.();
          this.onmessage?.({data:JSON.stringify({
            type:"snapshot",sequence:1,cols:80,screen_rows:24,
            terminal_scroll:true,palette:{background:"#0b1016",foreground:"#e6edf3"},
            rows:[{line_index:0,runs:[{text:"spotlight terminal",fg:"#e6edf3",bg:"#0b1016"}]}]
          })});
        },20);
      }
    }
    send(payload) {
      if(!this.workspace) spotlightFixture.agentMessages.push(JSON.parse(payload));
    }
    close() { this.readyState=3; }
    serverClose() { this.readyState=3;this.onclose?.({code:1006}); }
  };
})();
JS
)"

cdp Page.enable
cdp Emulation.setDeviceMetricsOverride '{"width":390,"height":844,"deviceScaleFactor":1,"mobile":true,"screenWidth":390,"screenHeight":844}'
cdp Page.addScriptToEvaluateOnNewDocument "$(jq -cn --argjson source "$fixture" '{source:$source}')"
cdp Page.navigate "$(jq -cn --arg url "$page_url" '{url:$url}')"
evaluate '(async()=>{
  for(let i=0;i<100&&!document.querySelector(".remote-shell__menu-button");i++) await new Promise(r=>setTimeout(r,50));
  document.querySelector(".remote-shell__menu-button").click();
  await new Promise(r=>setTimeout(r,100));
  document.querySelector(".remote-shell__drawer-footer button").click();
  for(let i=0;i<100&&!document.querySelector(".remote-shell--spotlight");i++) await new Promise(r=>setTimeout(r,50));
  const rows=[...document.querySelectorAll(".remote-shell__spotlight-agent")];
  if(rows.length!==2) throw new Error(`expected 2 spotlight agents, got ${rows.length}`);
  if(!rows[0].textContent.includes("Idle Agent")) throw new Error("idle Agent was not ordered first");
  if(document.querySelector(".remote-shell__header")) throw new Error("spotlight retained header");
  if(document.querySelector(".remote-shell__tabs")) throw new Error("spotlight retained title tabs");
  const terminal=document.querySelector(".remote-shell__terminal").getBoundingClientRect();
  if(terminal.top!==0) throw new Error(`terminal has extra title space: ${terminal.top}`);
  if(getComputedStyle(document.querySelector(".remote-shell__terminal-output")).borderTopWidth!=="0px") throw new Error("terminal kept decorative frame");
  if(getComputedStyle(document.querySelector(".remote-shell")).backgroundColor!=="rgb(11, 16, 22)") throw new Error("spotlight root is not dark");
})()'

evaluate '(()=>{const rect=document.querySelector(".remote-shell__terminal-output").getBoundingClientRect();return {x:Math.floor(rect.left+rect.width/2),top:Math.floor(rect.top+rect.height*0.7),bottom:Math.floor(rect.top+rect.height*0.3)}})()'
touch_x="$(jq -er '.result.result.value.x' <<<"$cdp_response")"
touch_top="$(jq -er '.result.result.value.top' <<<"$cdp_response")"
touch_bottom="$(jq -er '.result.result.value.bottom' <<<"$cdp_response")"
cdp Input.dispatchTouchEvent "$(jq -cn --argjson x "$touch_x" --argjson y "$touch_top" '{type:"touchStart",touchPoints:[{x:$x,y:$y}]}')"
cdp Input.dispatchTouchEvent "$(jq -cn --argjson x "$touch_x" --argjson y "$touch_bottom" '{type:"touchMove",touchPoints:[{x:$x,y:$y}]}')"
evaluate '(async()=>{await new Promise(requestAnimationFrame);if(!spotlightFixture.agentMessages.some(message=>message.type==="scroll"&&message.lines<0))throw new Error("mobile swipe up did not scroll before touchend")})()'
cdp Input.dispatchTouchEvent '{"type":"touchEnd","touchPoints":[]}'
evaluate '(async()=>{await new Promise(r=>setTimeout(r,180))})()'
cdp Input.dispatchTouchEvent "$(jq -cn --argjson x "$touch_x" --argjson y "$touch_bottom" '{type:"touchStart",touchPoints:[{x:$x,y:$y}]}')"
cdp Input.dispatchTouchEvent "$(jq -cn --argjson x "$touch_x" --argjson y "$touch_top" '{type:"touchMove",touchPoints:[{x:$x,y:$y}]}')"
evaluate '(async()=>{await new Promise(requestAnimationFrame);if(!spotlightFixture.agentMessages.some(message=>message.type==="scroll"&&message.lines>0))throw new Error("mobile swipe down did not scroll before touchend")})()'
cdp Input.dispatchTouchEvent '{"type":"touchEnd","touchPoints":[]}'
evaluate '(async()=>{
  spotlightFixture.swapActivity();
  await new Promise(r=>setTimeout(r,100));
  const rows=[...document.querySelectorAll(".remote-shell__spotlight-agent")];
  if(!rows[0].textContent.includes("Busy Agent")) throw new Error("live activity did not reorder tabs");
  const previous=spotlightFixture.workspaceConnections;
  spotlightFixture.workspaceSocket.serverClose();
  await new Promise(r=>setTimeout(r,700));
  if(spotlightFixture.workspaceConnections!==previous+1) throw new Error("workspace metadata did not reconnect");
  if(!document.querySelector(".remote-shell--spotlight")) throw new Error("reconnect exited spotlight mode");
})()'

if [ -n "$screenshot_path" ]; then
  cdp Page.captureScreenshot '{"format":"png","fromSurface":true}'
  jq -er '.result.data' <<<"$cdp_response" | base64 --decode >"$screenshot_path"
fi

printf 'PASS: Web spotlight, reconnect, and bidirectional mobile touch scrolling\n'
