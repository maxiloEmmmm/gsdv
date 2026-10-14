#!/usr/bin/env bash
# Uses an existing Chrome DevTools endpoint and served Web bundle.
# Example: bash tests/e2e/third_party/web_terminal_wheel.sh http://127.0.0.1:9447 http://127.0.0.1:4183
# HTTP/WS agent responses are mocked inside a new page; no real agent is resized.
set -euo pipefail

debug_url="${1:?Chrome DevTools HTTP endpoint required}"
page_url="${2:?Served GSDV Web URL required}"
target="$(curl --fail --silent --show-error -X PUT "$debug_url/json/new?about:blank")"
target_id="$(jq -er .id <<< "$target")"
target_ws="$(jq -er .webSocketDebuggerUrl <<< "$target")"
coproc WHEEL_CDP { websocat -t -B 4194304 "$target_ws"; }
cdp_pid="$WHEEL_CDP_PID"
exec {cdp_read}<&"${WHEEL_CDP[0]}"
exec {cdp_write}>&"${WHEEL_CDP[1]}"
request_id=0
cdp_response='{}'

# Releases only this check's page and connection. Example: exit -> close fixture.
cleanup() {
  curl --silent --max-time 2 "$debug_url/json/close/$target_id" >/dev/null || true
  kill "$cdp_pid" 2>/dev/null || true
  wait "$cdp_pid" 2>/dev/null || true
}
trap cleanup EXIT

# Waits for the matching CDP reply, skipping events. Example: Page.enable -> {}.
cdp() {
  request_id=$((request_id + 1))
  local request response params="${2-}"
  if [ -z "$params" ]; then params='{}'; fi
  request="$(jq -cn --argjson id "$request_id" --arg method "$1" \
    --argjson params "$params" '{id:$id,method:$method,params:$params}')"
  printf '%s\n' "$request" >&"$cdp_write"
  while IFS= read -r -t 10 response <&"$cdp_read"; do
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

# Evaluates browser assertions, including async rendering. Example: 1+1 -> 2.
evaluate() {
  cdp Runtime.evaluate "$(jq -cn --arg expression "$1" \
    '{expression:$expression,returnByValue:true,awaitPromise:true}')"
}

fixture="$(jq -Rs . <<'JS'
(() => {
  const workspace = {workspace_id:"fixture",name:"Wheel fixture",rows:[{
    row_index:0,cols:[{col_index:0,agents:[{agent_id:"a",title:"Agent A"}]}]
  }]};
  window.fetch = async () => new Response(JSON.stringify({workspaces:[workspace]}));
  window.wheelFixture = {
    // Captured terminal requests and current emulated terminal mode.
    messages: [], alternate: true, offset: 0, timer: 0, socket: null,
    // Emits one screen or normal scrollback. Example: alternate=true -> 12 rows.
    emit() {
      const count = this.alternate ? 12 : 500;
      this.socket.onmessage?.({data:JSON.stringify({
        type:"snapshot",sequence:++this.socket.sequence,cols:80,
        terminal_scroll:this.alternate,
        rows:Array.from({length:count},(_,i)=>({line_index:i-count+12,runs:[{
          text:`row ${i} offset ${this.offset}`,fg:"#111827",bg:"#ffffff"
        }]}))
      })});
    }
  };
  window.WebSocket = class {
    static OPEN = 1;
    readyState = 1;
    sequence = 0;
    // Opens a fixture connection. Example: new WebSocket -> initial snapshot.
    constructor() {
      wheelFixture.socket=this;
      setTimeout(()=>{this.onopen?.();wheelFixture.emit();},20);
    }
    // Responds to scroll input with a new visible screen. Example: +3 -> offset 3.
    send(raw) {
      const message=JSON.parse(raw);
      if(message.type!=="scroll") return;
      wheelFixture.messages.push(message);
      wheelFixture.offset+=message.lines;
      wheelFixture.emit();
    }
    // Closes only the mock socket. Example: close -> readyState 3.
    close() { this.readyState=3; }
  };
})();
JS
)"
cdp Page.enable
cdp Page.addScriptToEvaluateOnNewDocument "$(jq -cn --argjson source "$fixture" '{source:$source}')"
cdp Page.navigate "$(jq -cn --arg url "$page_url" '{url:$url}')"
evaluate '(async()=>{
  for(let i=0;i<100&&!document.querySelector(".remote-shell__menu-button");i++) await new Promise(r=>setTimeout(r,50));
  document.querySelector(".remote-shell__menu-button").click();
  await new Promise(r=>setTimeout(r,100));
  document.querySelector(".remote-shell__drawer-node--root").click();
  await new Promise(r=>setTimeout(r,100));
  document.querySelector(".remote-shell__agent-node").click();
  await new Promise(r=>setTimeout(r,400));
  const out=document.querySelector(".remote-shell__terminal-output");
  if(out.scrollHeight!==out.clientHeight) throw new Error("fixture should fit one screen");
})()'
cdp Input.dispatchMouseEvent '{"type":"mouseWheel","x":320,"y":240,"deltaX":0,"deltaY":-120}'
evaluate '(async()=>{
  await new Promise(r=>setTimeout(r,200));
  if(!(wheelFixture.messages.at(-1)?.lines>0)) throw new Error("up wheel never reached agent");
  if(!document.querySelector(".remote-shell__terminal-output").textContent.includes(`offset ${wheelFixture.offset}`)) throw new Error("scroll response was not rendered");
})()'
cdp Input.dispatchMouseEvent '{"type":"mouseWheel","x":320,"y":240,"deltaX":0,"deltaY":120}'
evaluate '(async()=>{
  await new Promise(r=>setTimeout(r,200));
  if(!(wheelFixture.messages.at(-1)?.lines<0)) throw new Error("down wheel never reached agent");
  wheelFixture.alternate=false;wheelFixture.emit();wheelFixture.messages=[];
  const out=document.querySelector(".remote-shell__terminal-output");
  out.scrollTop=out.scrollHeight;
  wheelFixture.timer=setInterval(()=>wheelFixture.emit(),30);
  await new Promise(r=>setTimeout(r,100));
})()'
cdp Input.synthesizeScrollGesture '{"x":320,"y":240,"yDistance":600,"speed":800,"gestureSourceType":"mouse"}'
evaluate '(async()=>{
  await new Promise(r=>setTimeout(r,200));
  clearInterval(wheelFixture.timer);
  const out=document.querySelector(".remote-shell__terminal-output");
  if(out.scrollHeight-out.clientHeight-out.scrollTop<500) throw new Error("streaming output blocked native scrolling");
  if(wheelFixture.messages.length) throw new Error("normal history sent PTY input");
})()'
printf 'PASS: interactive wheel forwarding, response rendering, streaming native history\n'
