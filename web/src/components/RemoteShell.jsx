import Button from "ant-design-vue/es/button";
import Drawer from "ant-design-vue/es/drawer";
import { defineComponent, onBeforeUnmount, onMounted, ref } from "vue";

/**
 * Limits browser-held terminal output to the agreed scrollback window.
 */
const MAX_TERMINAL_ROWS = 500;
const MAX_TERMINAL_COLS = 500;
const MAX_TERMINAL_VISIBLE_ROWS = 200;

/**
 * Renders the remote terminal page layout.
 *
 * This component owns the remote workspace tree, current agent binding, and
 * append-only terminal stream defined by the remote API document.
 */
export default defineComponent({
  name: "RemoteShell",

  /**
   * Creates state and event handlers for the remote workspace shell.
   */
  setup() {
    const drawerOpen = ref(false);
    const workspaceLoading = ref(false);
    const workspaceError = ref("");
    const workspaces = ref([]);
    const expandedTreeNodes = ref(new Set());
    const selectedAgent = ref(null);
    const terminalCols = ref(0);
    const terminalCursor = ref(null);
    const terminalSequence = ref(null);
    const terminalStatus = ref("Select an agent from the workspace drawer.");
    const terminalError = ref("");
    const draftInputElement = ref(null);
    const terminalOutputElement = ref(null);
    const imageInputElement = ref(null);
    const agentSocket = ref(null);
    const reconnectTimer = ref(0);
    const scrollFrame = ref(0);
    const lastTerminalResizeKey = ref("");
    let terminalRows = [];
    let terminalResizeHandler = null;

    /**
     * Loads workspace metadata from the documented remote HTTP route.
     */
    async function loadWorkspaceTree() {
      workspaceLoading.value = true;
      workspaceError.value = "";
      try {
        const response = await fetch("/api/workspace");
        if (!response.ok) {
          throw new Error(`workspace api failed: ${response.status}`);
        }
        const payload = await response.json();
        workspaces.value = Array.isArray(payload.workspaces)
          ? payload.workspaces
          : [];
      } catch (error) {
        workspaceError.value =
          error instanceof Error ? error.message : String(error);
      } finally {
        workspaceLoading.value = false;
      }
    }

    /**
     * Opens the workspace drawer from the header icon.
     */
    function openDrawer() {
      drawerOpen.value = true;
    }

    /**
     * Closes the workspace drawer from mask, close button, or model updates.
     */
    function closeDrawer() {
      drawerOpen.value = false;
    }

    /**
     * Mirrors drawer model updates emitted by ant-design-vue.
     */
    function updateDrawerOpen(open) {
      drawerOpen.value = open;
    }

    /**
     * Returns a stable tree id for the expandable drawer node.
     */
    function treeNodeId(...parts) {
      return parts.join(":");
    }

    /**
     * Checks whether the drawer tree node is currently expanded.
     */
    function treeNodeExpanded(id) {
      return expandedTreeNodes.value.has(id);
    }

    /**
     * Toggles a drawer tree node without mutating the Set in place.
     */
    function toggleTreeNode(id) {
      const next = new Set(expandedTreeNodes.value);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      expandedTreeNodes.value = next;
    }

    /**
     * Returns every expandable node id below one workspace.
     */
    function workspaceTreeNodeIds(workspace) {
      const ids = [treeNodeId(workspace.workspace_id)];
      const rows = Array.isArray(workspace.rows) ? workspace.rows : [];
      for (const row of rows) {
        ids.push(treeNodeId(workspace.workspace_id, row.row_index));
        const cols = Array.isArray(row.cols) ? row.cols : [];
        for (const col of cols) {
          ids.push(
            treeNodeId(workspace.workspace_id, row.row_index, col.col_index),
          );
        }
      }
      return ids;
    }

    /**
     * Expands one workspace to agents and collapses other workspaces.
     */
    function toggleWorkspaceTree(workspace) {
      const rootId = treeNodeId(workspace.workspace_id);
      if (treeNodeExpanded(rootId)) {
        expandedTreeNodes.value = new Set();
        return;
      }

      expandedTreeNodes.value = new Set(workspaceTreeNodeIds(workspace));
    }

    /**
     * Builds the documented WebSocket URL for one agent binding.
     */
    function agentOutputUrl(binding) {
      const params = new URLSearchParams({
        workspace_id: binding.workspace.workspace_id,
        row_index: String(binding.row.row_index),
        col_index: String(binding.col.col_index),
        agent_id: binding.agent.agent_id,
      });
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      return `${protocol}//${window.location.host}/api/agent/output/ws?${params}`;
    }

    /**
     * Clears terminal state before a fresh agent snapshot arrives.
     */
    function clearTerminal(message) {
      terminalRows = [];
      terminalCols.value = 0;
      terminalCursor.value = null;
      terminalSequence.value = null;
      terminalError.value = "";
      terminalStatus.value = message;
      renderTerminalEmpty(message);
    }

    /**
     * Closes the current WebSocket and cancels delayed reconnect work.
     */
    function closeAgentSocket() {
      if (reconnectTimer.value !== 0) {
        window.clearTimeout(reconnectTimer.value);
        reconnectTimer.value = 0;
      }
      if (agentSocket.value) {
        agentSocket.value.close();
        agentSocket.value = null;
      }
      lastTerminalResizeKey.value = "";
    }

    /**
     * Cancels pending terminal scroll work during component teardown.
     */
    function cancelTerminalScroll() {
      if (scrollFrame.value !== 0) {
        window.cancelAnimationFrame(scrollFrame.value);
        scrollFrame.value = 0;
      }
    }

    /**
     * Starts watching browser resize for remote PTY resize.
     */
    function observeTerminalOutputSize() {
      terminalResizeHandler = () => sendTerminalResize();
      window.addEventListener("resize", terminalResizeHandler);
      sendTerminalResize();
    }

    /**
     * Stops watching browser resize during component teardown.
     */
    function disconnectTerminalResizeObserver() {
      if (terminalResizeHandler) {
        window.removeEventListener("resize", terminalResizeHandler);
        terminalResizeHandler = null;
      }
    }

    /**
     * Measures one monospace cell from the terminal output style.
     */
    function measureTerminalCell(output) {
      const probe = document.createElement("span");
      probe.textContent = "M";
      probe.style.position = "absolute";
      probe.style.visibility = "hidden";
      probe.style.whiteSpace = "pre";
      probe.style.font = window.getComputedStyle(output).font;
      output.appendChild(probe);
      const rect = probe.getBoundingClientRect();
      probe.remove();
      const style = window.getComputedStyle(output);
      const lineHeight = Number.parseFloat(style.lineHeight);
      return {
        width: Math.max(1, rect.width),
        height: Math.max(1, Number.isFinite(lineHeight) ? lineHeight : rect.height),
      };
    }

    /**
     * Computes the browser-visible terminal grid size.
     */
    function currentTerminalGridSize() {
      const output = terminalOutputElement.value;
      if (!output) {
        return null;
      }

      const style = window.getComputedStyle(output);
      const paddingLeft = Number.parseFloat(style.paddingLeft) || 0;
      const paddingRight = Number.parseFloat(style.paddingRight) || 0;
      const paddingTop = Number.parseFloat(style.paddingTop) || 0;
      const paddingBottom = Number.parseFloat(style.paddingBottom) || 0;
      const availableWidth = output.clientWidth - paddingLeft - paddingRight;
      const availableHeight = output.clientHeight - paddingTop - paddingBottom;
      const cell = measureTerminalCell(output);
      const cols = Math.min(
        MAX_TERMINAL_COLS,
        Math.max(1, Math.floor(availableWidth / cell.width)),
      );
      const rows = Math.min(
        MAX_TERMINAL_VISIBLE_ROWS,
        Math.max(1, Math.floor(availableHeight / cell.height)),
      );
      return { cols, rows };
    }

    /**
     * Sends browser terminal dimensions to the remote agent WebSocket.
     */
    function sendTerminalResize(socket = agentSocket.value) {
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        return;
      }

      const size = currentTerminalGridSize();
      if (!size) {
        return;
      }

      const key = `${size.cols}x${size.rows}`;
      if (key === lastTerminalResizeKey.value) {
        return;
      }
      lastTerminalResizeKey.value = key;
      socket.send(JSON.stringify({ type: "resize", ...size }));
    }

    /**
     * Keeps only the newest rows in the browser terminal buffer.
     */
    function trimTerminalRows(rows) {
      if (rows.length <= MAX_TERMINAL_ROWS) {
        return rows;
      }

      // 触发条件：WebSocket append 长时间持续推送。
      // 不能直接无限保存：浏览器状态和 DOM 会持续膨胀。
      // 防止回归：remote 页面长时间打开后滚动和渲染变卡。
      return rows.slice(rows.length - MAX_TERMINAL_ROWS);
    }

    /**
     * Scrolls the terminal output after Vue has rendered new rows.
     */
    function scrollTerminalToBottom() {
      cancelTerminalScroll();
      scrollFrame.value = window.requestAnimationFrame(() => {
        scrollFrame.value = 0;
        const output = terminalOutputElement.value;
        if (output) {
          output.scrollTop = output.scrollHeight;
        }
      });
    }

    /**
     * Connects the browser to the selected agent output WebSocket.
     */
    function connectAgentSocket(binding) {
      closeAgentSocket();
      clearTerminal("Connecting to agent terminal...");

      const socket = new WebSocket(agentOutputUrl(binding));
      agentSocket.value = socket;

      socket.onopen = () => {
        if (agentSocket.value === socket) {
          terminalStatus.value = "Waiting for terminal snapshot...";
          sendTerminalResize(socket);
        }
      };

      socket.onmessage = (event) => {
        if (agentSocket.value !== socket) {
          return;
        }
        handleSocketMessage(socket, binding, event.data);
      };

      socket.onerror = () => {
        if (agentSocket.value === socket) {
          terminalError.value = "Agent terminal connection failed.";
        }
      };

      socket.onclose = () => {
        if (agentSocket.value === socket) {
          terminalStatus.value = "Agent terminal disconnected.";
        }
      };
    }

    /**
     * Handles one JSON message from the documented agent output stream.
     */
    function handleSocketMessage(socket, binding, rawMessage) {
      let message;
      try {
        message = JSON.parse(rawMessage);
      } catch (_error) {
        terminalError.value = "Received invalid terminal message.";
        return;
      }

      if (message.type === "snapshot") {
        applySnapshotMessage(message);
        return;
      }

      if (message.type === "append") {
        applyAppendMessage(binding, message);
        return;
      }

      if (message.type === "ping") {
        socket.send(JSON.stringify({ type: "pong", sequence: message.sequence }));
      }
    }

    /**
     * Applies a full terminal snapshot from a newly opened connection.
     */
    function applySnapshotMessage(message) {
      const rows = Array.isArray(message.rows) ? message.rows : [];
      terminalRows = trimTerminalRows(rows);
      terminalCols.value = Number.isFinite(message.cols) ? message.cols : 0;
      terminalCursor.value = message.cursor ?? null;
      terminalSequence.value = Number.isFinite(message.sequence)
        ? message.sequence
        : null;
      terminalStatus.value = "Agent terminal connected.";
      terminalError.value = "";
      renderTerminalRows(terminalRows);
      scrollTerminalToBottom();
    }

    /**
     * Appends new terminal rows when the sequence is contiguous.
     */
    function applyAppendMessage(binding, message) {
      const nextSequence = message.sequence;
      const currentSequence = terminalSequence.value;
      if (
        !Number.isFinite(nextSequence) ||
        !Number.isFinite(currentSequence) ||
        nextSequence !== currentSequence + 1
      ) {
        reconnectCurrentAgent(binding);
        return;
      }

      const rows = Array.isArray(message.rows) ? message.rows : [];
      terminalRows = trimTerminalRows(terminalRows.concat(rows));
      terminalSequence.value = nextSequence;
      terminalCursor.value = message.cursor ?? terminalCursor.value;
      renderTerminalRows(terminalRows);
      scrollTerminalToBottom();
    }

    /**
     * Reconnects after a sequence gap so the server can send a fresh snapshot.
     */
    function reconnectCurrentAgent(binding) {
      terminalError.value = "Terminal stream sequence changed; reconnecting.";
      closeAgentSocket();
      // 触发条件：append sequence 不是当前 sequence + 1。
      // 不能直接继续 append：缺失的行无法从 append-only 协议恢复。
      // 防止回归：输出丢包后页面仍显示为貌似正常的半截日志。
      reconnectTimer.value = window.setTimeout(() => {
        reconnectTimer.value = 0;
        if (selectedAgent.value === binding) {
          connectAgentSocket(binding);
        }
      }, 300);
    }

    /**
     * Selects an agent from the drawer and binds terminal output to it.
     */
    function selectAgent(workspace, row, col, agent) {
      const binding = { workspace, row, col, agent };
      selectedAgent.value = binding;
      drawerOpen.value = false;
      connectAgentSocket(binding);
    }

    /**
     * Builds the shared agent operation body for remote API calls.
     */
    function agentRequestBody(binding) {
      return {
        workspace_id: binding.workspace.workspace_id,
        row_index: binding.row.row_index,
        col_index: binding.col.col_index,
        agent_id: binding.agent.agent_id,
      };
    }

    /**
     * Handles Enter submit while leaving Shift+Enter for textarea newlines.
     */
    function handleDraftKeydown(event) {
      if (event.key !== "Enter" || event.shiftKey) {
        return;
      }

      event.preventDefault();
      submitDraft();
    }

    /**
     * Sends the current input text through the documented agent input route.
     */
    async function submitDraft() {
      const binding = selectedAgent.value;
      const input = draftInputElement.value;
      const text = input?.value.trim() ?? "";

      if (!binding || text.length === 0) {
        return;
      }

      const body = {
        ...agentRequestBody(binding),
        text,
      };

      try {
        const response = await fetch("/api/agent/input", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        });
        if (!response.ok) {
          throw new Error(`input api failed: ${response.status}`);
        }
        input.value = "";
      } catch (error) {
        terminalError.value =
          error instanceof Error ? error.message : String(error);
      }
    }

    /**
     * Sends an interrupt request to the selected agent.
     */
    async function interruptAgent() {
      const binding = selectedAgent.value;
      if (!binding) {
        return;
      }

      try {
        const response = await fetch("/api/agent/esc", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(agentRequestBody(binding)),
        });
        if (!response.ok) {
          throw new Error(`esc api failed: ${response.status}`);
        }
      } catch (error) {
        terminalError.value =
          error instanceof Error ? error.message : String(error);
      }
    }

    /**
     * Opens the local image picker for the selected agent.
     */
    function openImagePicker() {
      if (!selectedAgent.value || !imageInputElement.value) {
        return;
      }

      imageInputElement.value.click();
    }

    /**
     * Sends the selected image file through the remote image route.
     */
    async function submitImageFile(file) {
      const binding = selectedAgent.value;
      if (!binding || !file) {
        return;
      }

      try {
        const image = await readImageFile(file);
        const response = await fetch("/api/agent/image", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            ...agentRequestBody(binding),
            image_base64: image.base64,
            mime_type: image.mimeType,
          }),
        });
        if (!response.ok) {
          throw new Error(`image api failed: ${response.status}`);
        }
      } catch (error) {
        terminalError.value =
          error instanceof Error ? error.message : String(error);
      }
    }

    /**
     * Handles image input changes and resets the input for repeat picks.
     */
    function handleImageChange(event) {
      const input = event.target;
      const file = input.files?.[0] ?? null;
      submitImageFile(file);
      input.value = "";
    }

    /**
     * Reads a browser image file into API-ready base64 payload fields.
     */
    function readImageFile(file) {
      return new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onerror = () => reject(new Error("image file read failed"));
        reader.onload = () => {
          const parsed = parseImageDataUrl(reader.result);
          if (parsed) {
            resolve(parsed);
          } else {
            reject(new Error("invalid image data url"));
          }
        };
        reader.readAsDataURL(file);
      });
    }

    /**
     * Parses a FileReader data URL without inventing a MIME fallback.
     */
    function parseImageDataUrl(value) {
      if (typeof value !== "string") {
        return null;
      }

      const match = value.match(/^data:([^;]+);base64,(.*)$/);
      if (!match) {
        return null;
      }

      return {
        mimeType: match[1],
        base64: match[2],
      };
    }

    /**
     * Returns display text for one terminal cell.
     */
    function terminalCellText(cell) {
      return cell.hidden ? " " : cell.text || " ";
    }

    /**
     * Builds a style descriptor for one terminal cell.
     */
    function terminalCellStyle(cell) {
      const fg = cell.fg || "#d1fae5";
      const bg = cell.bg || "#0f172a";
      return {
        color: cell.inverse ? bg : fg,
        backgroundColor: cell.inverse ? fg : bg,
        bold: Boolean(cell.bold),
        italic: Boolean(cell.italic),
        dim: Boolean(cell.dim),
        underline: cell.underline || "none",
        strikeout: Boolean(cell.strikeout),
      };
    }

    /**
     * Checks whether two terminal style descriptors can share one DOM node.
     */
    function terminalStyleEqual(left, right) {
      return (
        left.color === right.color &&
        left.backgroundColor === right.backgroundColor &&
        left.bold === right.bold &&
        left.italic === right.italic &&
        left.dim === right.dim &&
        left.underline === right.underline &&
        left.strikeout === right.strikeout
      );
    }

    /**
     * Coalesces terminal cells into styled text runs.
     */
    function terminalRuns(cells) {
      const runs = [];
      for (const cell of cells) {
        const style = terminalCellStyle(cell);
        const text = terminalCellText(cell);
        const last = runs[runs.length - 1];
        if (last && terminalStyleEqual(last, style)) {
          last.text += text;
          continue;
        }
        runs.push({ ...style, text });
      }
      return runs;
    }

    /**
     * Renders one merged terminal text run.
     */
    function renderTerminalRun(run, index) {
      const fg = run.fg || run.color || "#d1fae5";
      const bg = run.bg || run.backgroundColor || "#0f172a";
      const color = run.inverse ? bg : fg;
      const backgroundColor = run.inverse ? fg : bg;
      return (
        <span
          class={{
            "remote-shell__terminal-run": true,
            "remote-shell__terminal-run--bold": run.bold,
            "remote-shell__terminal-run--italic": run.italic,
            "remote-shell__terminal-run--dim": run.dim,
            "remote-shell__terminal-run--strike": run.strikeout,
          }}
          style={{
            color,
            backgroundColor,
            textDecorationLine: terminalTextDecoration(run),
            textDecorationStyle: terminalTextDecorationStyle(run),
          }}
          key={index}
        >
          {run.text}
        </span>
      );
    }

    /**
     * Creates one terminal run DOM node outside Vue render.
     */
    function terminalRunNode(run) {
      const node = document.createElement("span");
      const fg = run.fg || run.color || "#d1fae5";
      const bg = run.bg || run.backgroundColor || "#0f172a";
      node.className = [
        "remote-shell__terminal-run",
        run.bold ? "remote-shell__terminal-run--bold" : "",
        run.italic ? "remote-shell__terminal-run--italic" : "",
        run.dim ? "remote-shell__terminal-run--dim" : "",
        run.strikeout ? "remote-shell__terminal-run--strike" : "",
      ]
        .filter(Boolean)
        .join(" ");
      node.style.color = run.inverse ? bg : fg;
      node.style.backgroundColor = run.inverse ? fg : bg;
      node.style.textDecorationLine = terminalTextDecoration(run);
      node.style.textDecorationStyle = terminalTextDecorationStyle(run);
      node.textContent = run.text;
      return node;
    }

    /**
     * Creates one terminal row DOM node outside Vue render.
     */
    function terminalRowNode(row) {
      const node = document.createElement("div");
      node.className = row.wrapped
        ? "remote-shell__terminal-row remote-shell__terminal-row--wrapped"
        : "remote-shell__terminal-row";
      const cells = Array.isArray(row.cells) ? row.cells : [];
      const runs = Array.isArray(row.runs) ? row.runs : terminalRuns(cells);
      for (const run of runs) {
        node.appendChild(terminalRunNode(run));
      }
      return node;
    }

    /**
     * Renders terminal rows imperatively so Vue parent renders cannot touch them.
     */
    function renderTerminalRows(rows) {
      const output = terminalOutputElement.value;
      if (!output) {
        return;
      }

      const fragment = document.createDocumentFragment();
      for (const row of rows) {
        fragment.appendChild(terminalRowNode(row));
      }
      output.replaceChildren(fragment);
    }

    /**
     * Renders terminal empty/status text outside Vue render.
     */
    function renderTerminalEmpty(message) {
      const output = terminalOutputElement.value;
      if (!output) {
        return;
      }

      const node = document.createElement("div");
      node.className = "remote-shell__terminal-empty";
      node.textContent = message;
      output.replaceChildren(node);
    }

    /**
     * Returns the CSS underline line value for one terminal cell.
     */
    function terminalTextDecoration(cell) {
      const lines = [];
      if (cell.underline && cell.underline !== "none") {
        lines.push("underline");
      }
      if (cell.strikeout) {
        lines.push("line-through");
      }
      return lines.length === 0 ? "none" : lines.join(" ");
    }

    /**
     * Returns the CSS underline style for one terminal cell.
     */
    function terminalTextDecorationStyle(cell) {
      if (cell.underline === "curly") {
        return "wavy";
      }
      if (cell.underline === "dotted" || cell.underline === "dashed") {
        return cell.underline;
      }
      if (cell.underline === "double") {
        return "double";
      }
      return "solid";
    }

    /**
     * Checks whether a drawer agent row matches the active binding.
     */
    function agentSelected(workspace, row, col, agent) {
      const binding = selectedAgent.value;
      return (
        binding?.workspace.workspace_id === workspace.workspace_id &&
        binding?.row.row_index === row.row_index &&
        binding?.col.col_index === col.col_index &&
        binding?.agent.agent_id === agent.agent_id
      );
    }

    /**
     * Renders one terminal row from structured cells.
     */
    function renderTerminalRow(row) {
      const cells = Array.isArray(row.cells) ? row.cells : [];
      const runs = Array.isArray(row.runs) ? row.runs : terminalRuns(cells);
      return (
        <div
          class={{
            "remote-shell__terminal-row": true,
            "remote-shell__terminal-row--wrapped": row.wrapped,
          }}
          key={row.line_index}
        >
          {runs.map(renderTerminalRun)}
        </div>
      );
    }

    /**
     * Renders a drawer tree toggle row.
     */
    function renderTreeToggle(label, id, depth, extraClass, onClick) {
      const expanded = treeNodeExpanded(id);
      return (
        <button
          class={["remote-shell__drawer-node", extraClass]}
          style={{ paddingLeft: `${8 + depth * 16}px` }}
          type="button"
          aria-expanded={expanded}
          onClick={onClick ?? (() => toggleTreeNode(id))}
        >
          <span class="remote-shell__drawer-caret" aria-hidden="true">
            {expanded ? "v" : ">"}
          </span>
          <span class="remote-shell__drawer-label">{label}</span>
        </button>
      );
    }

    /**
     * Renders all agents in one column tree node.
     */
    function renderAgents(workspace, row, col) {
      const agents = Array.isArray(col.agents) ? col.agents : [];
      return agents.map((agent) => (
        <button
          class={{
            "remote-shell__agent-node": true,
            "remote-shell__agent-node--selected": agentSelected(
              workspace,
              row,
              col,
              agent,
            ),
          }}
          style={{ paddingLeft: "56px" }}
          type="button"
          key={agent.agent_id}
          onClick={() => selectAgent(workspace, row, col, agent)}
        >
          {agent.title}
        </button>
      ));
    }

    /**
     * Renders one column branch from the workspace tree.
     */
    function renderColumn(workspace, row, col) {
      const id = treeNodeId(
        workspace.workspace_id,
        row.row_index,
        col.col_index,
      );
      return (
        <div class="remote-shell__drawer-branch" key={id}>
          {renderTreeToggle(`col ${col.col_index}`, id, 2)}
          {treeNodeExpanded(id) ? renderAgents(workspace, row, col) : null}
        </div>
      );
    }

    /**
     * Renders one row branch from the workspace tree.
     */
    function renderRow(workspace, row) {
      const id = treeNodeId(workspace.workspace_id, row.row_index);
      const cols = Array.isArray(row.cols) ? row.cols : [];
      return (
        <div class="remote-shell__drawer-branch" key={id}>
          {renderTreeToggle(`row ${row.row_index}`, id, 1)}
          {treeNodeExpanded(id)
            ? cols.map((col) => renderColumn(workspace, row, col))
            : null}
        </div>
      );
    }

    /**
     * Renders one workspace branch from the workspace tree.
     */
    function renderWorkspace(workspace) {
      const id = treeNodeId(workspace.workspace_id);
      const rows = Array.isArray(workspace.rows) ? workspace.rows : [];
      return (
        <div class="remote-shell__drawer-branch" key={workspace.workspace_id}>
          {renderTreeToggle(
            workspace.name,
            id,
            0,
            "remote-shell__drawer-node--root",
            () => toggleWorkspaceTree(workspace),
          )}
          {treeNodeExpanded(id)
            ? rows.map((row) => renderRow(workspace, row))
            : null}
        </div>
      );
    }

    /**
     * Releases browser resources owned by the remote shell.
     */
    function cleanupRemoteShell() {
      closeAgentSocket();
      cancelTerminalScroll();
      disconnectTerminalResizeObserver();
    }

    onMounted(() => {
      loadWorkspaceTree();
      renderTerminalEmpty(terminalStatus.value);
      observeTerminalOutputSize();
    });
    onBeforeUnmount(cleanupRemoteShell);

    /**
     * Renders the remote shell with drawer, terminal, and composer.
     */
    return () => (
      <div class="remote-shell">
        <header class="remote-shell__header">
          <Button
            class="remote-shell__menu-button"
            aria-label="Open workspaces"
            onClick={openDrawer}
          >
            <span class="remote-shell__menu-icon" aria-hidden="true">
              <span />
              <span />
              <span />
            </span>
          </Button>
          <div class="remote-shell__heading">
            <div class="remote-shell__title">GSDV Remote</div>
            <div class="remote-shell__subtitle">
              {selectedAgent.value
                ? `${selectedAgent.value.workspace.name} / ${selectedAgent.value.agent.title}`
                : "No agent selected"}
            </div>
          </div>
        </header>

        <main class="remote-shell__terminal" aria-label="Agent terminal output">
          <div class="remote-shell__terminal-output" ref={terminalOutputElement} />
          {terminalError.value ? (
            <div class="remote-shell__terminal-banner">
              {terminalError.value}
            </div>
          ) : null}
        </main>

        <footer class="remote-shell__composer">
          <Button
            class="remote-shell__action-button"
            aria-label="Interrupt agent"
            disabled={!selectedAgent.value}
            onClick={interruptAgent}
          >
            ESC
          </Button>
          <Button
            class="remote-shell__action-button"
            aria-label="Select image"
            disabled={!selectedAgent.value}
            onClick={openImagePicker}
          >
            IMG
          </Button>
          <textarea
            class="remote-shell__input"
            aria-label="Agent input"
            ref={draftInputElement}
            disabled={!selectedAgent.value}
            placeholder="Send input to the selected agent"
            rows="1"
            onKeydown={handleDraftKeydown}
          />
          <Button
            class="remote-shell__send-button"
            aria-label="Send input"
            type="primary"
            disabled={!selectedAgent.value}
            onClick={submitDraft}
          >
            &gt;
          </Button>
          <input
            class="remote-shell__image-input"
            ref={imageInputElement}
            type="file"
            accept="image/*"
            onChange={handleImageChange}
          />
        </footer>

        <Drawer
          title="Workspaces"
          placement="left"
          open={drawerOpen.value}
          width={320}
          onClose={closeDrawer}
          {...{ "onUpdate:open": updateDrawerOpen }}
        >
          <nav class="remote-shell__drawer-tree" aria-label="Workspace tree">
            {workspaceLoading.value ? (
              <div class="remote-shell__drawer-state">Loading workspaces...</div>
            ) : null}
            {workspaceError.value ? (
              <div class="remote-shell__drawer-state remote-shell__drawer-state--error">
                {workspaceError.value}
              </div>
            ) : null}
            {!workspaceLoading.value &&
            !workspaceError.value &&
            workspaces.value.length === 0 ? (
              <div class="remote-shell__drawer-state">No workspaces</div>
            ) : null}
            {workspaces.value.map(renderWorkspace)}
          </nav>
        </Drawer>
      </div>
    );
  },
});
