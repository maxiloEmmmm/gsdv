import Button from "ant-design-vue/es/button";
import Drawer from "ant-design-vue/es/drawer";
import { useVirtualizer } from "@tanstack/vue-virtual";
import {
  computed,
  defineComponent,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
} from "vue";
import { sessionActivity, sessionEvents } from "../sessionEvents";
import { withSessionMarkdown } from "../sessionMarkdown";
import { renderSessionMermaid } from "../sessionMermaid";

/**
 * Limits browser-held terminal output to the agreed scrollback window.
 */
const TERMINAL_BOTTOM_THRESHOLD_PX = 2;
const AGENT_RECONNECT_BASE_DELAY_MS = 400;
const AGENT_RECONNECT_MAX_DELAY_MS = 8000;
const WORKSPACE_RECONNECT_BASE_DELAY_MS = 500;
const WORKSPACE_RECONNECT_MAX_DELAY_MS = 8000;
const REMOTE_SHELL_VIEWPORT_HEIGHT_VAR = "--remote-shell-viewport-height";
const SESSION_ROW_ESTIMATE_PX = 72;
const SESSION_ROW_OVERSCAN = 6;
/**
 * Fallback palette used before the first Rust terminal snapshot arrives.
 */
const DEFAULT_TERMINAL_PALETTE = {
  foreground: "#e5e7eb",
  background: "#0b0d10",
  cursor: "#f8fafc",
  muted: "#8b929c",
  primary: "#3b82f6",
};

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
    const spotlightMode = ref(false);
    const spotlightRankMode = ref(false);
    const spotlightSidebarCollapsed = ref(false);
    const keyMenuOpen = ref(false);
    const expandedTreeNodes = ref(new Set());
    const tabs = ref([]);
    const activeTabId = ref(null);
    const draftInputElement = ref(null);
    const terminalOutputElement = ref(null);
    const imageInputElement = ref(null);
    const scrollFrame = ref(0);
    let visualViewportHandler = null;
    let visualViewportSyncTimers = [];
    let workspaceSocket = null;
    let workspaceReconnectTimer = 0;
    let workspaceReconnectAttempt = 0;
    let workspaceSocketClosing = false;
    let spotlightLeaderId = null;

    /**
     * Returns render rows only after the initial session snapshot is complete.
     *
     * Example: 500 MiB history at seek 20 MiB -> no message VDOM yet.
     */
    const sessionRows = computed(() => {
      const tab = activeTab();
      if (!tab || tab.initialLoading) {
        return [];
      }
      const rows = sessionRenderRows(
        tab.events,
        tabDisplayActivity(tab) === "busy",
      );
      if (tab.historyStartSeek > 0) {
        rows.unshift({
          kind: "history_loader",
          id: `history-loader:${tab.sessionId}:${tab.historyStartSeek}`,
          loading: tab.historyLoading,
          error: tab.historyError,
        });
      }
      return rows;
    });

    /**
     * Virtualizes variable-height session rows against the terminal viewport.
     *
     * Example: 20,000 history rows -> mount only the visible rows plus overscan.
     */
    const sessionVirtualizer = useVirtualizer(
      computed(() => ({
        count: sessionRows.value.length,
        getScrollElement: () => terminalOutputElement.value,
        estimateSize: () => SESSION_ROW_ESTIMATE_PX,
        getItemKey: (index) =>
          `${activeTabId.value ?? "none"}:${sessionRows.value[index]?.id ?? index}`,
        overscan: SESSION_ROW_OVERSCAN,
      })),
    );

    /**
     * Returns the small set of rows currently mounted by the virtualizer.
     *
     * Example: viewport around row 100 -> rows 94 through 112.
     */
    const visibleSessionRows = computed(() =>
      sessionVirtualizer.value.getVirtualItems(),
    );

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
        applyWorkspaceSnapshot(payload);
      } catch (error) {
        workspaceError.value =
          error instanceof Error ? error.message : String(error);
      } finally {
        workspaceLoading.value = false;
      }
    }

    /**
     * Builds the live workspace metadata WebSocket URL.
     */
    function workspaceOutputUrl() {
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      return `${protocol}//${window.location.host}/api/workspace/ws`;
    }

    /**
     * Applies one server workspace snapshot and repairs spotlight selection.
     */
    function applyWorkspaceSnapshot(payload) {
      const nextRankMode = payload?.spotlight_rank_mode === true;
      if (spotlightRankMode.value !== nextRankMode) {
        spotlightLeaderId = null;
      }
      spotlightRankMode.value = nextRankMode;
      workspaces.value = Array.isArray(payload?.workspaces)
        ? payload.workspaces
        : [];
      refreshOpenTabBindings();
      if (spotlightMode.value) {
        if (spotlightRankMode.value) {
          reconcileSpotlightLeader();
        } else {
          ensureSpotlightSelection();
        }
      }
    }

    /**
     * Opens the metadata stream used by live spotlight ordering.
     */
    function connectWorkspaceSocket() {
      closeWorkspaceSocket(false);
      workspaceSocketClosing = false;
      const socket = new WebSocket(workspaceOutputUrl());
      workspaceSocket = socket;
      socket.onopen = () => {
        if (workspaceSocket === socket) {
          workspaceReconnectAttempt = 0;
          workspaceError.value = "";
        }
      };
      socket.onmessage = (event) => {
        if (workspaceSocket !== socket) {
          return;
        }
        try {
          applyWorkspaceSnapshot(JSON.parse(event.data));
        } catch (_error) {
          workspaceError.value = "Received invalid workspace metadata.";
        }
      };
      socket.onerror = () => {
        if (workspaceSocket === socket) {
          workspaceError.value = "Workspace metadata connection failed.";
        }
      };
      socket.onclose = () => {
        if (workspaceSocket !== socket) {
          return;
        }
        workspaceSocket = null;
        if (!workspaceSocketClosing) {
          scheduleWorkspaceReconnect();
        }
      };
    }

    /**
     * Closes the workspace stream and optionally suppresses reconnect.
     */
    function closeWorkspaceSocket(closing = true) {
      workspaceSocketClosing = closing;
      if (workspaceReconnectTimer !== 0) {
        window.clearTimeout(workspaceReconnectTimer);
        workspaceReconnectTimer = 0;
      }
      if (workspaceSocket) {
        const socket = workspaceSocket;
        workspaceSocket = null;
        socket.close();
      }
    }

    /**
     * Reconnects workspace metadata with bounded exponential backoff.
     */
    function scheduleWorkspaceReconnect() {
      if (workspaceSocketClosing || workspaceReconnectTimer !== 0) {
        return;
      }
      const delay = Math.min(
        WORKSPACE_RECONNECT_MAX_DELAY_MS,
        WORKSPACE_RECONNECT_BASE_DELAY_MS * 2 ** workspaceReconnectAttempt,
      );
      workspaceReconnectAttempt = Math.min(workspaceReconnectAttempt + 1, 8);
      workspaceReconnectTimer = window.setTimeout(() => {
        workspaceReconnectTimer = 0;
        if (!workspaceSocketClosing) {
          connectWorkspaceSocket();
        }
      }, delay);
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
    function agentOutputUrl(tab) {
      const binding = tab.binding;
      const params = new URLSearchParams({
        workspace_id: binding.workspace.workspace_id,
        row_index: String(binding.row.row_index),
        col_index: String(binding.col.col_index),
        agent_id: binding.agent.agent_id,
        session_id: tab.sessionId,
        seek: String(tab.seek),
      });
      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      return `${protocol}//${window.location.host}/api/agent/session/ws?${params}`;
    }

    /**
     * Returns a stable browser tab id for one agent binding.
     */
    function agentTabId(workspace, row, col, agent) {
      return [
        workspace.workspace_id,
        row.row_index,
        col.col_index,
        agent.agent_id,
      ].join(":");
    }

    /**
     * Returns each current Agent binding once, independent of grid duplication.
     */
    function allAgentBindings() {
      const bindings = [];
      const seen = new Set();
      for (const workspace of workspaces.value) {
        for (const row of Array.isArray(workspace.rows) ? workspace.rows : []) {
          for (const col of Array.isArray(row.cols) ? row.cols : []) {
            for (const agent of Array.isArray(col.agents) ? col.agents : []) {
              const id = agentTabId(workspace, row, col, agent);
              const stableId = `${workspace.workspace_id}:${agent.agent_id}`;
              if (seen.has(stableId)) {
                continue;
              }
              seen.add(stableId);
              bindings.push({ id, workspace, row, col, agent });
            }
          }
        }
      }
      return bindings;
    }

    /**
     * Returns persisted spotlight Agents with stopped/error entries first.
     */
    function spotlightAgentBindings() {
      return allAgentBindings()
        .filter((binding) => Number.isInteger(binding.agent.spotlight_level))
        .sort((left, right) => {
          const rank =
            spotlightAgentRank(left.agent) - spotlightAgentRank(right.agent);
          if (rank !== 0) {
            return rank;
          }
          const level =
            left.agent.spotlight_level - right.agent.spotlight_level;
          if (level !== 0) {
            return level;
          }
          return (
            spotlightStoppedOrder(left.agent) -
            spotlightStoppedOrder(right.agent)
          );
        });
    }

    /**
     * Maps server activity to the stable spotlight ordering group.
     */
    function spotlightAgentRank(agent) {
      if (
        agent.manually_stopped === true ||
        agent.activity === "idle" ||
        agent.activity === "error"
      ) {
        return 0;
      }
      return agent.activity === "busy" ? 2 : 1;
    }

    /**
     * Returns a stopped Agent's runtime order or a stable trailing fallback.
     */
    function spotlightStoppedOrder(agent) {
      return Number.isFinite(agent.stopped_order)
        ? agent.stopped_order
        : Number.MAX_SAFE_INTEGER;
    }

    /**
     * Returns the stable cross-workspace identity used for leader changes.
     */
    function spotlightBindingId(binding) {
      return `${binding.workspace.workspace_id}:${binding.agent.agent_id}`;
    }

    /**
     * Returns the final workspace path segment for compact spotlight tabs.
     */
    function spotlightWorkspaceLeaf(name) {
      const segments = String(name ?? "")
        .split(/[\\/]/)
        .filter(Boolean);
      return segments.at(-1) ?? "";
    }

    /**
     * Refreshes metadata references held by already-open terminal tabs.
     */
    function refreshOpenTabBindings() {
      const current = new Map(
        allAgentBindings().map((binding) => [binding.id, binding]),
      );
      for (const tab of tabs.value) {
        const binding = current.get(tab.id);
        if (binding) {
          tab.binding = binding;
          tab.title = binding.agent.title;
        }
      }
      refreshTabs();
    }

    /**
     * Opens the read-only Web spotlight browser.
     */
    function enterSpotlightMode() {
      spotlightMode.value = true;
      drawerOpen.value = false;
      spotlightLeaderId = null;
      if (spotlightRankMode.value) {
        reconcileSpotlightLeader();
      } else {
        ensureSpotlightSelection();
      }
    }

    /**
     * Returns to the ordinary remote workspace browser.
     */
    function exitSpotlightMode() {
      spotlightMode.value = false;
    }

    /**
     * Persists the Web rank-mode checkbox and applies it immediately.
     */
    async function updateSpotlightRankMode(event) {
      const enabled = event.target.checked === true;
      const previous = spotlightRankMode.value;
      spotlightRankMode.value = enabled;
      spotlightLeaderId = null;
      if (enabled) {
        reconcileSpotlightLeader();
      }
      try {
        const response = await fetch("/api/spotlight/rank-mode", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ enabled }),
        });
        if (!response.ok) {
          throw new Error(`spotlight rank mode failed: ${response.status}`);
        }
      } catch (error) {
        spotlightRankMode.value = previous;
        spotlightLeaderId = null;
        workspaceError.value =
          error instanceof Error ? error.message : String(error);
      }
    }

    /**
     * Switches only when rank mode observes a different first Agent.
     */
    function reconcileSpotlightLeader() {
      const bindings = spotlightAgentBindings();
      const first = bindings.find(
        (binding) => spotlightAgentRank(binding.agent) === 0,
      ) ?? null;
      const nextLeaderId = first ? spotlightBindingId(first) : null;
      if (!first) {
        spotlightLeaderId = null;
        ensureSpotlightSelection();
        return;
      }
      if (spotlightLeaderId === nextLeaderId) {
        ensureSpotlightSelection();
        return;
      }
      spotlightLeaderId = nextLeaderId;
      if (first) {
        selectAgent(first.workspace, first.row, first.col, first.agent);
      } else {
        activeTabId.value = null;
        renderActiveTerminal();
      }
    }

    /**
     * Keeps the selected Web terminal bound to a current spotlight member.
     */
    function ensureSpotlightSelection() {
      const bindings = spotlightAgentBindings();
      const current = activeBinding();
      if (
        current &&
        bindings.some(
          (binding) =>
            binding.workspace.workspace_id === current.workspace.workspace_id &&
            binding.agent.agent_id === current.agent.agent_id,
        )
      ) {
        return;
      }
      const first = bindings[0];
      if (first) {
        selectAgent(first.workspace, first.row, first.col, first.agent);
      } else {
        activeTabId.value = null;
        renderActiveTerminal();
      }
    }

    /**
     * Returns the currently visible remote agent tab.
     */
    function activeTab() {
      return tabs.value.find((tab) => tab.id === activeTabId.value) ?? null;
    }

    /**
     * Returns the currently visible agent binding.
     */
    function activeBinding() {
      return activeTab()?.binding ?? null;
    }

    /**
     * Returns the visible runtime state for one Agent.
     *
     * Example: manually stopped busy Agent -> stopped.
     */
    function agentDisplayActivity(agent, persistedActivity = null) {
      if (agent?.manually_stopped === true) {
        return "stopped";
      }
      if (["busy", "idle", "error"].includes(persistedActivity)) {
        return persistedActivity;
      }
      const activity = agent?.activity;
      return ["busy", "idle", "error"].includes(activity)
        ? activity
        : "unknown";
    }

    /**
     * Returns one open tab's session-derived activity with hook state as fallback.
     *
     * Example: stale hook plus task_started record -> busy.
     */
    function tabDisplayActivity(tab) {
      return agentDisplayActivity(
        tab?.binding?.agent,
        tab?.sessionActivity ?? null,
      );
    }

    /**
     * Forces Vue to re-render after mutating tab fields in place.
     */
    function refreshTabs() {
      tabs.value = tabs.value.slice();
    }

    /**
     * Creates a browser tab state for one remote agent binding.
     */
    function createAgentTab(workspace, row, col, agent) {
      const binding = { workspace, row, col, agent };
      return {
        id: agentTabId(workspace, row, col, agent),
        binding,
        title: agent.title,
        events: [],
        eventIndexes: new Map(),
        eventSerial: 0,
        sessionActivity: null,
        sessionId: agent.session_id ?? "",
        // A successful /clear hides the current session until its replacement arrives.
        awaitingSessionReset: false,
        // The old session is retained so reconnects cannot refill cleared history.
        clearedSessionId: null,
        agentKind: agent.agent_kind ?? "codex",
        seek: 0,
        // Earliest browser-held byte; older pages move only this cursor.
        historyStartSeek: 0,
        historyLoading: false,
        historyError: "",
        initialEndSeek: 0,
        initialLoading: true,
        hasLoadedInitialSession: false,
        decoder: new TextDecoder(),
        lineBuffer: "",
        palette: { ...DEFAULT_TERMINAL_PALETTE },
        status: "Connecting to agent session...",
        error: "",
        draft: "",
        socket: null,
        reconnectTimer: 0,
        reconnectAttempt: 0,
        renderFrame: 0,
        closing: false,
        // Whether ordinary terminal history follows new output.
        followOutput: true,
        // Last native scroll position saved for this tab.
        scrollTop: 0,
      };
    }

    /**
     * Checks whether a tab object is still present in the tab strip.
     */
    function tabStillOpen(tab) {
      return tabs.value.some((candidate) => candidate.id === tab.id);
    }

    /**
     * Clears terminal state before a fresh agent snapshot arrives.
     */
    function clearTerminal(tab, message) {
      tab.events = [];
      tab.eventIndexes = new Map();
      tab.eventSerial = 0;
      tab.sessionActivity = null;
      tab.decoder = new TextDecoder();
      tab.lineBuffer = "";
      tab.seek = 0;
      tab.historyStartSeek = 0;
      tab.historyLoading = false;
      tab.historyError = "";
      tab.initialEndSeek = 0;
      tab.initialLoading = true;
      tab.hasLoadedInitialSession = false;
      tab.followOutput = true;
      tab.scrollTop = 0;
      tab.error = "";
      tab.status = message;
      refreshTabs();
    }

    /**
     * Clears rendered history while preserving the current stream cursor.
     *
     * Example: successful /clear at seek 42 -> empty view still reading from 42.
     */
    function clearSessionView(tab, message) {
      tab.events = [];
      tab.eventIndexes = new Map();
      tab.eventSerial = 0;
      tab.sessionActivity = null;
      tab.decoder = new TextDecoder();
      tab.lineBuffer = "";
      tab.historyStartSeek = 0;
      tab.historyLoading = false;
      tab.historyError = "";
      tab.initialLoading = false;
      tab.hasLoadedInitialSession = true;
      tab.followOutput = true;
      tab.scrollTop = 0;
      tab.error = "";
      tab.status = message;
      refreshTabs();
    }

    /**
     * Closes one WebSocket and cancels delayed reconnect work.
     */
    function closeAgentSocket(tab, closing = false) {
      if (!tab) {
        return;
      }
      if (tab.reconnectTimer !== 0) {
        window.clearTimeout(tab.reconnectTimer);
        tab.reconnectTimer = 0;
      }
      if (tab.renderFrame !== 0) {
        window.cancelAnimationFrame(tab.renderFrame);
        tab.renderFrame = 0;
      }
      tab.historyLoading = false;
      tab.closing = closing;
      if (tab.socket) {
        tab.socket.close();
        tab.socket = null;
      }
      refreshTabs();
    }

    /**
     * Closes every open agent tab during component teardown.
     */
    function closeAllAgentSockets() {
      for (const tab of tabs.value) {
        closeAgentSocket(tab, true);
      }
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
     * Drops queued interactive wheel input when its tab or socket is leaving.
     * Example: close active tab -> no delayed scroll reaches the next agent.
     */
    function cancelInteractiveTerminalScroll(tab = terminalWheelTab) {
      if (terminalWheelFrame !== 0) {
        window.cancelAnimationFrame(terminalWheelFrame);
        terminalWheelFrame = 0;
      }
      if (tab) {
        if (tab.wheelGestureTimer !== 0) {
          window.clearTimeout(tab.wheelGestureTimer);
          tab.wheelGestureTimer = 0;
        }
        tab.pendingWheelLines = 0;
        tab.pendingWheelCell = null;
        tab.wheelGestureActive = false;
      }
      terminalWheelTab = null;
      terminalTouchGesture = null;
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
     * Syncs the shell height with the browser visual viewport.
     */
    function syncVisualViewportHeight() {
      const viewport = window.visualViewport;
      const height = viewport ? viewport.height : window.innerHeight;
      document.documentElement.style.setProperty(
        REMOTE_SHELL_VIEWPORT_HEIGHT_VAR,
        `${Math.max(1, Math.floor(height))}px`,
      );
      window.scrollTo(0, 0);
    }

    /**
     * Clears delayed visual viewport sync work.
     */
    function clearVisualViewportSyncTimers() {
      for (const timer of visualViewportSyncTimers) {
        window.clearTimeout(timer);
      }
      visualViewportSyncTimers = [];
    }

    /**
     * Re-syncs mobile viewport height across iOS keyboard animation frames.
     */
    function scheduleVisualViewportSync() {
      clearVisualViewportSyncTimers();
      syncVisualViewportHeight();
      window.requestAnimationFrame(syncVisualViewportHeight);
      visualViewportSyncTimers = [80, 220, 480].map((delay) =>
        window.setTimeout(() => {
          syncVisualViewportHeight();
        }, delay),
      );
    }

    /**
     * Watches mobile keyboard viewport changes for fixed shell layout.
     */
    function observeVisualViewportSize() {
      visualViewportHandler = () => {
        scheduleVisualViewportSync();
      };
      scheduleVisualViewportSync();
      window.addEventListener("resize", visualViewportHandler);
      if (window.visualViewport) {
        window.visualViewport.addEventListener("resize", visualViewportHandler);
        window.visualViewport.addEventListener("scroll", visualViewportHandler);
      }
    }

    /**
     * Stops watching mobile visual viewport changes during teardown.
     */
    function disconnectVisualViewportObserver() {
      if (!visualViewportHandler) {
        return;
      }
      window.removeEventListener("resize", visualViewportHandler);
      if (window.visualViewport) {
        window.visualViewport.removeEventListener("resize", visualViewportHandler);
        window.visualViewport.removeEventListener("scroll", visualViewportHandler);
      }
      document.documentElement.style.removeProperty(
        REMOTE_SHELL_VIEWPORT_HEIGHT_VAR,
      );
      clearVisualViewportSyncTimers();
      visualViewportHandler = null;
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
    function sendTerminalResize(tab = activeTab(), socket = tab?.socket) {
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        return;
      }

      const size = currentTerminalGridSize();
      if (!size) {
        return;
      }

      const key = `${size.cols}x${size.rows}`;
      if (key === tab.lastResizeKey) {
        return;
      }
      tab.lastResizeKey = key;
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
     * Example: a following tab receives output -> scroll its viewport to bottom.
     */
    function scrollTerminalToBottom(tab = activeTab()) {
      cancelTerminalScroll();
      scrollFrame.value = window.requestAnimationFrame(() => {
        const output = terminalOutputElement.value;
        if (output && tab?.followOutput && activeTabId.value === tab.id) {
          const lastIndex = sessionRows.value.length - 1;
          if (lastIndex >= 0) {
            sessionVirtualizer.value.scrollToIndex(lastIndex, { align: "end" });
          }
          output.scrollTop = output.scrollHeight;
          tab.scrollTop = output.scrollTop;
          tab.followOutput = true;
        }
        // Dynamic row measurements settle after the first paint. Pin once more
        // so opening a large session starts at its real, measured bottom.
        scrollFrame.value = window.requestAnimationFrame(() => {
          scrollFrame.value = 0;
          const settledOutput = terminalOutputElement.value;
          if (
            settledOutput &&
            tab?.followOutput &&
            activeTabId.value === tab.id
          ) {
            settledOutput.scrollTop = settledOutput.scrollHeight;
            tab.scrollTop = settledOutput.scrollTop;
          }
        });
      });
    }

    /**
     * Checks whether the terminal viewport is currently at the bottom.
     * Example: remaining scroll distance 1px -> true.
     */
    function terminalViewportAtBottom(output) {
      if (output.scrollHeight <= output.clientHeight) {
        return true;
      }
      return (
        output.scrollHeight - output.scrollTop - output.clientHeight <=
        TERMINAL_BOTTOM_THRESHOLD_PX
      );
    }

    /**
     * Checks whether new output for one tab should remain pinned to the bottom.
     * Example: a tab paused by wheel-up -> false.
     */
    function terminalShouldFollowOutput(tab) {
      return tab?.followOutput ?? true;
    }

    /**
     * Records native terminal scrolling and resumes following at the bottom.
     * Example: scrollTop 120 above the bottom -> save 120 and pause following.
     */
    function handleTerminalScroll() {
      const output = terminalOutputElement.value;
      const tab = activeTab();
      if (!output || !tab) {
        return;
      }
      tab.scrollTop = output.scrollTop;
      tab.followOutput = terminalViewportAtBottom(output);
      if (!tab.followOutput) {
        cancelTerminalScroll();
      }
    }

    /**
     * Resolves a browser pointer to the one-based PTY cell under it.
     * Example: bottom-right pointer in an 80x24 terminal -> col 80, row 24.
     */
    function terminalPointerCell(event, output, tab) {
      const rect = output.getBoundingClientRect();
      const style = window.getComputedStyle(output);
      const paddingLeft = Number.parseFloat(style.paddingLeft) || 0;
      const paddingRight = Number.parseFloat(style.paddingRight) || 0;
      const paddingTop = Number.parseFloat(style.paddingTop) || 0;
      const paddingBottom = Number.parseFloat(style.paddingBottom) || 0;
      const left = rect.left + paddingLeft;
      const top = rect.top + paddingTop;
      const width = Math.max(
        1,
        output.clientWidth - paddingLeft - paddingRight,
      );
      const height = Math.max(
        1,
        output.clientHeight - paddingTop - paddingBottom,
      );
      const cols = Math.max(1, tab.cols || 1);
      const rows = Math.max(1, tab.gridRows || tab.rows.length || 1);
      const x = Math.min(
        width - Number.EPSILON,
        Math.max(0, event.clientX - left),
      );
      const y = Math.min(
        height - Number.EPSILON,
        Math.max(0, event.clientY - top),
      );
      return {
        col: Math.min(cols, Math.floor((x / width) * cols) + 1),
        row: Math.min(rows, Math.floor((y / height) * rows) + 1),
      };
    }

    /**
     * Sends at most one accumulated interactive scroll request per paint frame.
     * Example: twelve 0.2-line events -> one 2-line request with 0.4 retained.
     */
    function flushInteractiveTerminalScroll(tab) {
      terminalWheelFrame = 0;
      terminalWheelTab = null;
      if (
        activeTabId.value !== tab.id ||
        !tab.terminalScroll ||
        tab.socket?.readyState !== WebSocket.OPEN
      ) {
        tab.pendingWheelLines = 0;
        tab.pendingWheelCell = null;
        return;
      }
      const wholeLines = Math.trunc(tab.pendingWheelLines);
      if (wholeLines === 0) {
        terminalWheelTab = tab;
        return;
      }
      const lines =
        Math.sign(wholeLines) *
        Math.min(MAX_TERMINAL_SCROLL_LINES, Math.abs(wholeLines));
      tab.pendingWheelLines -= lines;
      const cell = tab.pendingWheelCell;
      tab.socket.send(JSON.stringify({ type: "scroll", lines, ...cell }));
      if (Math.abs(tab.pendingWheelLines) >= 1) {
        scheduleInteractiveTerminalScroll(tab);
      }
    }

    /**
     * Coalesces interactive wheel work onto the next browser paint frame.
     * Example: repeated trackpad events in one frame -> one WebSocket message.
     */
    function scheduleInteractiveTerminalScroll(tab) {
      if (terminalWheelFrame !== 0) {
        return;
      }
      terminalWheelTab = tab;
      terminalWheelFrame = window.requestAnimationFrame(() =>
        flushInteractiveTerminalScroll(tab),
      );
    }

    /**
     * Accumulates one wheel event while guaranteeing immediate gesture feedback.
     * Example: first 0.2-line event -> one line now; later events stay fractional.
     */
    function queueInteractiveTerminalScroll(tab, deltaLines, pointerCell) {
      if (tab.wheelGestureTimer !== 0) {
        window.clearTimeout(tab.wheelGestureTimer);
      }
      if (!tab.wheelGestureActive && Math.abs(deltaLines) < 1) {
        // 触控板或高分辨率鼠标的首个 delta 可能远小于一行。
        // 等累计满一行才发送会表现为“手停后才动”，每个事件都取整又会过快。
        tab.pendingWheelLines = -Math.sign(deltaLines);
      } else {
        tab.pendingWheelLines -= deltaLines;
      }
      tab.wheelGestureActive = true;
      tab.pendingWheelCell = pointerCell;
      tab.wheelGestureTimer = window.setTimeout(() => {
        tab.wheelGestureTimer = 0;
        tab.wheelGestureActive = false;
        if (Math.abs(tab.pendingWheelLines) < 1) {
          tab.pendingWheelLines = 0;
        }
      }, TERMINAL_WHEEL_GESTURE_IDLE_MS);
      scheduleInteractiveTerminalScroll(tab);
    }

    /**
     * Routes interactive wheel input to the agent, or scrolls local history.
     * Example: terminal deltaMode=line, deltaY=-3 -> send scroll lines=3.
     */
    function handleTerminalWheel(event) {
      if (event.ctrlKey || !Number.isFinite(event.deltaY) || event.deltaY === 0) {
        return;
      }
      const tab = activeTab();
      const output = terminalOutputElement.value;
      if (tab?.terminalScroll && output) {
        if (tab.socket?.readyState !== WebSocket.OPEN) {
          return;
        }
        // Interactive TUIs expose only one screen, so native DOM scrolling has
        // no history to move. Forward wheel lines using the desktop PTY path.
        event.preventDefault();
        const measuredLineHeight = Number.parseFloat(
          window.getComputedStyle(output).lineHeight,
        );
        const lineHeight =
          Number.isFinite(measuredLineHeight) && measuredLineHeight > 0
            ? measuredLineHeight
            : 1;
        let deltaLines = event.deltaY;
        if (event.deltaMode !== WheelEvent.DOM_DELTA_LINE) {
          deltaLines /= lineHeight;
          if (event.deltaMode === WheelEvent.DOM_DELTA_PAGE) {
            deltaLines *= output.clientHeight;
          }
        }
        // 触控板会产生不足一行的高频事件。逐条取整会放大输入并让
        // 每个事件触发一次整屏 snapshot，因此保留小数并按帧合并。
        queueInteractiveTerminalScroll(
          tab,
          deltaLines,
          terminalPointerCell(event, output, tab),
        );
        return;
      }
      if (event.deltaY > 0) {
        return;
      }
      if (tab) {
        tab.followOutput = false;
      }
      cancelTerminalScroll();
    }

    /**
     * Stops delayed auto-scroll before a mobile swipe can move the viewport.
     * Example: touchstart at the bottom -> do not snap back during swipe-up.
     */
    function handleTerminalTouchStart(event) {
      const tab = activeTab();
      if (tab) {
        tab.followOutput = false;
      }
      cancelTerminalScroll();
      if (!tab?.terminalScroll || event.touches.length !== 1) {
        terminalTouchGesture = null;
        return;
      }
      const touch = event.touches[0];
      terminalTouchGesture = {
        identifier: touch.identifier,
        clientX: touch.clientX,
        clientY: touch.clientY,
      };
      // Interactive TUI screens have no DOM history. Prevent native page panning
      // so every finger delta follows the same low-latency PTY scroll path as wheel.
      event.preventDefault();
    }

    /**
     * Converts an interactive mobile swipe into PTY scroll lines.
     * Example: finger moves up by one line -> send one downward terminal scroll line.
     */
    function handleTerminalTouchMove(event) {
      const tab = activeTab();
      const output = terminalOutputElement.value;
      if (!tab?.terminalScroll || !output || !terminalTouchGesture) {
        return;
      }
      const touch = Array.from(event.touches).find(
        (candidate) => candidate.identifier === terminalTouchGesture.identifier,
      );
      if (!touch || tab.socket?.readyState !== WebSocket.OPEN) {
        return;
      }
      event.preventDefault();
      const measuredLineHeight = Number.parseFloat(
        window.getComputedStyle(output).lineHeight,
      );
      const lineHeight =
        Number.isFinite(measuredLineHeight) && measuredLineHeight > 0
          ? measuredLineHeight
          : 1;
      const deltaLines =
        (terminalTouchGesture.clientY - touch.clientY) / lineHeight;
      terminalTouchGesture.clientX = touch.clientX;
      terminalTouchGesture.clientY = touch.clientY;
      if (deltaLines !== 0) {
        queueInteractiveTerminalScroll(
          tab,
          deltaLines,
          terminalPointerCell(touch, output, tab),
        );
      }
    }

    /**
     * Reconciles terminal follow state after a touch gesture finishes.
     * Example: touchend back at the bottom -> resume following new output.
     */
    function handleTerminalTouchEnd() {
      terminalTouchGesture = null;
      handleTerminalScroll();
    }

    /**
     * Restores a tab's history position after its terminal DOM is redrawn.
     * Example: scrollTop 240 before snapshot -> scrollTop 240 after snapshot.
     */
    function restoreTerminalScroll(tab, scrollTop = tab?.scrollTop ?? 0) {
      cancelTerminalScroll();
      const output = terminalOutputElement.value;
      if (!output || !tab || activeTabId.value !== tab.id) {
        return;
      }
      output.scrollTop = scrollTop;
      tab.scrollTop = output.scrollTop;
    }

    /**
     * Connects one browser tab to its append-only session stream.
     *
     * Example: reconnect at seek 4096 -> server resumes from byte 4096.
     */
    function connectAgentSocket(tab, preserveSession = false) {
      closeAgentSocket(tab);
      tab.closing = false;
      if (preserveSession && (tab.events.length > 0 || tab.seek > 0)) {
        tab.status = "Reconnecting to agent session...";
        tab.error = "";
        refreshTabs();
      } else {
        clearTerminal(tab, "Connecting to agent session...");
      }

      const socket = new WebSocket(agentOutputUrl(tab));
      socket.binaryType = "arraybuffer";
      tab.socket = socket;
      refreshTabs();

      socket.onopen = () => {
        if (tab.socket === socket && tabStillOpen(tab)) {
          tab.status = "Loading agent session...";
          tab.error = "";
          refreshTabs();
        }
      };

      socket.onmessage = (event) => {
        if (tab.socket !== socket || !tabStillOpen(tab)) {
          return;
        }
        handleSessionSocketMessage(tab, event.data);
      };

      socket.onerror = () => {
        if (tab.socket === socket && tabStillOpen(tab)) {
          tab.error = "Agent session connection failed.";
          refreshTabs();
        }
      };

      socket.onclose = () => {
        if (tab.socket === socket && tabStillOpen(tab)) {
          tab.socket = null;
          scheduleAgentReconnect(tab, "Agent session disconnected.");
        }
      };
    }

    /**
     * Schedules bounded exponential reconnect while the browser tab stays open.
     * Example: three failed sockets -> retry after 400ms, 800ms, then 1600ms.
     */
    function scheduleAgentReconnect(tab, reason) {
      if (
        tab.closing ||
        !tabStillOpen(tab) ||
        tab.reconnectTimer !== 0
      ) {
        return;
      }
      const exponent = Math.min(tab.reconnectAttempt, 8);
      const delay = Math.min(
        AGENT_RECONNECT_MAX_DELAY_MS,
        AGENT_RECONNECT_BASE_DELAY_MS * 2 ** exponent,
      );
      tab.reconnectAttempt += 1;
      tab.status = "Reconnecting to agent session...";
      tab.error = `${reason} Retrying...`;
      tab.reconnectTimer = window.setTimeout(() => {
        tab.reconnectTimer = 0;
        if (!tab.closing && tabStillOpen(tab)) {
          connectAgentSocket(tab, true);
        }
      }, delay);
      refreshTabs();
    }

    /**
     * Handles one text reset or binary JSONL range from the session stream.
     *
     * Example: 16-byte header plus JSONL -> validate seek, parse, render locally.
     */
    function handleSessionSocketMessage(tab, rawMessage) {
      if (typeof rawMessage === "string") {
        applySessionReset(tab, rawMessage);
        return;
      }
      if (!(rawMessage instanceof ArrayBuffer) || rawMessage.byteLength < 16) {
        reconnectCurrentAgent(tab, "Invalid session frame.");
        return;
      }
      const view = new DataView(rawMessage);
      const fromSeek = Number(view.getBigUint64(0, true));
      const toSeek = Number(view.getBigUint64(8, true));
      const validRange =
        Number.isSafeInteger(fromSeek) &&
        Number.isSafeInteger(toSeek) &&
        toSeek >= fromSeek;
      const isHistoryRange =
        validRange &&
        tab.historyLoading &&
        toSeek === tab.historyStartSeek;
      const payload = new Uint8Array(rawMessage, 16);
      if (isHistoryRange) {
        prependSessionBytes(tab, payload, fromSeek);
        return;
      }
      if (!validRange || fromSeek !== tab.seek) {
        reconnectCurrentAgent(tab, "Session range changed.");
        return;
      }
      appendSessionBytes(tab, payload, toSeek);
    }

    /**
     * Applies provider/session metadata and clears only on a real session switch.
     *
     * Example: reset from session a to b -> discard a and restart at seek 0.
     */
    function applySessionReset(tab, rawMessage) {
      let message;
      try {
        message = JSON.parse(rawMessage);
      } catch (_error) {
        reconnectCurrentAgent(tab, "Invalid session metadata.");
        return;
      }
      if (message.type !== "session_reset") {
        return;
      }
      const sessionId = String(message.session_id ?? "");
      const seek = Number(message.seek ?? 0);
      const availableSeek = Number(message.available_seek ?? seek);
      const resetsSessionView =
        sessionId !== tab.sessionId ||
        !Number.isSafeInteger(seek) ||
        seek !== tab.seek;
      if (resetsSessionView) {
        clearTerminal(tab, "Loading agent session...");
        tab.sessionId = sessionId;
        tab.seek = Number.isSafeInteger(seek) ? seek : 0;
        tab.historyStartSeek = tab.seek;
      }
      if (
        tab.awaitingSessionReset &&
        sessionId.length > 0 &&
        sessionId !== tab.clearedSessionId
      ) {
        tab.awaitingSessionReset = false;
        tab.clearedSessionId = null;
      }
      tab.initialEndSeek = Number.isSafeInteger(availableSeek)
        ? Math.max(tab.seek, availableSeek)
        : tab.seek;
      tab.initialLoading =
        !tab.hasLoadedInitialSession &&
        sessionId.length > 0 &&
        tab.seek < tab.initialEndSeek;
      if (!tab.initialLoading) {
        tab.hasLoadedInitialSession = true;
      }
      tab.agentKind = message.agent_kind === "claude" ? "claude" : "codex";
      tab.reconnectAttempt = 0;
      tab.status = tab.initialLoading
        ? "Loading agent session..."
        : sessionId
          ? "Agent session connected."
          : "Waiting for session id...";
      tab.error = "";
      refreshTabs();
    }

    /**
     * Decodes complete JSONL records while retaining a split trailing line.
     *
     * Example: chunk ending mid-record -> keep suffix until the next chunk.
     */
    function appendSessionBytes(tab, bytes, toSeek) {
      if (
        tab.awaitingSessionReset &&
        tab.sessionId === tab.clearedSessionId
      ) {
        // /clear does not create its replacement session until the next prompt.
        // Keep the transport cursor current without restoring the old history.
        tab.seek = toSeek;
        tab.status = "Waiting for the next prompt...";
        tab.error = "";
        refreshTabs();
        return;
      }
      const isActive = activeTabId.value === tab.id;
      const initialLoading = tab.initialLoading;
      const followOutput = isActive ? terminalShouldFollowOutput(tab) : false;
      const decoded = tab.decoder.decode(bytes, { stream: true });
      const lines = `${tab.lineBuffer}${decoded}`.split("\n");
      tab.lineBuffer = lines.pop() ?? "";
      for (const line of lines) {
        appendSessionLine(tab, line);
      }
      tab.seek = toSeek;
      if (initialLoading && tab.seek < tab.initialEndSeek) {
        // Initial history may span thousands of frames. Parsing without a Vue
        // refresh prevents a growing full-session VDOM and scrolling animation.
        tab.status = "Loading agent session...";
        return;
      }
      if (initialLoading) {
        tab.initialLoading = false;
        tab.hasLoadedInitialSession = true;
        tab.followOutput = true;
      }
      tab.status = "Agent session connected.";
      tab.error = "";
      scheduleSessionRender(tab, initialLoading || followOutput);
    }

    /**
     * Requests the preceding 100 complete records for the active session.
     *
     * Example: earliest byte 4096 -> request the JSONL page ending at 4096.
     */
    function loadEarlierSession(tab = activeTab()) {
      if (!tab || tab.historyLoading || tab.historyStartSeek <= 0) {
        return;
      }
      if (tab.socket?.readyState !== WebSocket.OPEN) {
        tab.historyError = "Session connection is unavailable.";
        refreshTabs();
        return;
      }
      tab.historyLoading = true;
      tab.historyError = "";
      tab.socket.send(
        JSON.stringify({
          type: "load_history",
          session_id: tab.sessionId,
          before_seek: tab.historyStartSeek,
        }),
      );
      refreshTabs();
    }

    /**
     * Prepends one complete history page while keeping the current content anchored.
     *
     * Example: range [2048, 4096) -> move only historyStartSeek to 2048.
     */
    function prependSessionBytes(tab, bytes, fromSeek) {
      const output =
        activeTabId.value === tab.id ? terminalOutputElement.value : null;
      const anchorRow = sessionRows.value.find(
        (row) => row.kind !== "history_loader",
      );
      const anchorElement = anchorRow
        ? [...document.querySelectorAll(".remote-shell__session-virtual-row")].find(
            (element) => element.dataset.rowKey === anchorRow.id,
          )
        : null;
      const anchorOffset =
        output && anchorElement
          ? anchorElement.getBoundingClientRect().top -
            output.getBoundingClientRect().top
          : 0;
      const historyState = {
        agentKind: tab.agentKind,
        events: [],
        eventIndexes: new Map(),
        eventSerial: tab.eventSerial,
        sessionActivity: null,
      };
      const decoded = new TextDecoder().decode(bytes);
      for (const line of decoded.split("\n")) {
        // Historical task state is stale by definition and must not replace
        // the current busy/idle state derived from the live tail.
        appendSessionLine(historyState, line, false);
      }
      tab.eventSerial = historyState.eventSerial;
      // Stable provider ids can span page boundaries. Keep the newer copy so
      // loading history cannot roll a completed event back to an older form.
      const olderEvents = historyState.events.filter(
        (event) => !tab.eventIndexes.has(event.id),
      );
      tab.events = olderEvents.concat(tab.events);
      tab.eventIndexes = new Map(
        tab.events.map((event, index) => [event.id, index]),
      );
      tab.historyStartSeek = fromSeek;
      tab.historyLoading = false;
      tab.historyError = "";
      refreshTabs();

      if (!output) {
        return;
      }
      nextTick(() => {
        const anchorIndex = sessionRows.value.findIndex(
          (row) => row.id === anchorRow?.id,
        );
        if (anchorIndex < 0) {
          return;
        }
        // Virtual row estimates change as older rows mount. Anchoring the same
        // message twice prevents those measurements from moving the viewport.
        sessionVirtualizer.value.scrollToIndex(anchorIndex, { align: "start" });
        const correctAnchor = () => {
          const currentOutput = terminalOutputElement.value;
          if (activeTabId.value !== tab.id || !currentOutput) {
            return;
          }
          const currentAnchor = [
            ...document.querySelectorAll(
              ".remote-shell__session-virtual-row",
            ),
          ].find((element) => element.dataset.rowKey === anchorRow.id);
          if (!currentAnchor) {
            return;
          }
          const currentOffset =
            currentAnchor.getBoundingClientRect().top -
            currentOutput.getBoundingClientRect().top;
          currentOutput.scrollTop += currentOffset - anchorOffset;
          tab.scrollTop = currentOutput.scrollTop;
          tab.followOutput = false;
        };
        window.requestAnimationFrame(() => {
          correctAnchor();
          window.requestAnimationFrame(correctAnchor);
        });
      });
    }

    /**
     * Coalesces large initial session loads to at most one Vue render per frame.
     *
     * Example: ten 256 KiB frames arrive together -> one DOM update.
     */
    function scheduleSessionRender(tab, followOutput) {
      if (tab.renderFrame !== 0) {
        return;
      }
      tab.renderFrame = window.requestAnimationFrame(() => {
        tab.renderFrame = 0;
        refreshTabs();
        nextTick(async () => {
          if (followOutput) {
            scrollTerminalToBottom(tab);
          }
          const rendered = await renderSessionMermaid(
            terminalOutputElement.value,
          );
          if (
            rendered &&
            followOutput &&
            activeTabId.value === tab.id &&
            terminalShouldFollowOutput(tab)
          ) {
            scrollTerminalToBottom(tab);
          }
        });
      });
    }

    /**
     * Parses and upserts the visible events produced by one JSONL record.
     *
     * Example: duplicate tool id -> replace its existing rendered event.
     */
    function appendSessionLine(tab, line, updateActivity = true) {
      if (!line.trim()) {
        return;
      }
      let record;
      try {
        record = JSON.parse(line);
      } catch (_error) {
        return;
      }
      const fallbackKey = `line:${tab.eventSerial}`;
      tab.eventSerial += 1;
      const nextActivity = sessionActivity(tab.agentKind, record);
      if (updateActivity && nextActivity) {
        tab.sessionActivity = nextActivity;
      }
      for (const event of sessionEvents(tab.agentKind, record, fallbackKey)) {
        const renderedEvent = withSessionMarkdown(event);
        if (!renderedEvent) {
          continue;
        }
        const key = `${renderedEvent.kind}:${renderedEvent.key}`;
        const index = tab.eventIndexes.get(key);
        if (index === undefined) {
          tab.eventIndexes.set(key, tab.events.length);
          tab.events.push({ ...renderedEvent, id: key });
        } else {
          tab.events[index] = { ...renderedEvent, id: key };
        }
      }
    }

    /**
     * Reconnects from the browser's last complete byte range after a gap.
     *
     * Example: expected seek 4096, received 8192 -> resume again at 4096.
     */
    function reconnectCurrentAgent(tab, reason) {
      closeAgentSocket(tab);
      scheduleAgentReconnect(tab, reason);
    }

    /**
     * Selects an agent from the drawer and binds terminal output to it.
     */
    function selectAgent(workspace, row, col, agent) {
      const tabId = agentTabId(workspace, row, col, agent);
      const existing = tabs.value.find((tab) => tab.id === tabId);
      drawerOpen.value = false;
      if (existing) {
        activateAgentTab(existing.id);
        return;
      }

      const tab = createAgentTab(workspace, row, col, agent);
      tabs.value = tabs.value.concat(tab);
      activateAgentTab(tab.id);
      connectAgentSocket(tab);
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
     * Stores the current composer draft on the active browser tab.
     */
    function handleDraftInput(event) {
      const tab = activeTab();
      if (tab) {
        tab.draft = event.target.value;
      }
    }

    /**
     * Sends the current input text through the documented agent input route.
     */
    async function submitDraft() {
      const tab = activeTab();
      const binding = tab?.binding ?? null;
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
        tab.draft = "";
        if (text === "/clear") {
          tab.awaitingSessionReset = true;
          tab.clearedSessionId = tab.sessionId;
          clearSessionView(tab, "Waiting for the next prompt...");
        }
      } catch (error) {
        tab.error = error instanceof Error ? error.message : String(error);
        refreshTabs();
      }
    }

    /**
     * Toggles the compact terminal key menu beside the composer.
     */
    function toggleKeyMenu() {
      keyMenuOpen.value = !keyMenuOpen.value;
    }

    /**
     * Sends one whitelisted terminal key through the active output socket.
     */
    function sendAgentKey(key) {
      const tab = activeTab();
      keyMenuOpen.value = false;
      if (!tab || tab.socket?.readyState !== WebSocket.OPEN) {
        if (tab) {
          tab.error = "Agent terminal is not connected.";
          refreshTabs();
        }
        return;
      }
      tab.socket.send(JSON.stringify({ type: "key", key }));
    }

    /**
     * Opens the local image picker for the selected agent.
     */
    function openImagePicker() {
      if (!activeTab() || !imageInputElement.value) {
        return;
      }

      imageInputElement.value.click();
    }

    /**
     * Sends the selected image file through the remote image route.
     */
    async function submitImageFile(file) {
      const tab = activeTab();
      const binding = tab?.binding ?? null;
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
        tab.error = error instanceof Error ? error.message : String(error);
        refreshTabs();
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
     * Normalizes a server-provided terminal palette with stable fallbacks.
     */
    function terminalPalette(value) {
      const palette = value && typeof value === "object" ? value : {};
      return {
        foreground:
          typeof palette.foreground === "string"
            ? palette.foreground
            : DEFAULT_TERMINAL_PALETTE.foreground,
        background:
          typeof palette.background === "string"
            ? palette.background
            : DEFAULT_TERMINAL_PALETTE.background,
        cursor:
          typeof palette.cursor === "string"
            ? palette.cursor
            : DEFAULT_TERMINAL_PALETTE.cursor,
        muted:
          typeof palette.muted === "string"
            ? palette.muted
            : DEFAULT_TERMINAL_PALETTE.muted,
        primary:
          typeof palette.primary === "string"
            ? palette.primary
            : DEFAULT_TERMINAL_PALETTE.primary,
      };
    }

    /**
     * Returns the palette used by the currently rendered terminal tab.
     */
    function activeTerminalPalette() {
      return activeTab()?.palette ?? DEFAULT_TERMINAL_PALETTE;
    }

    /**
     * Exposes the active terminal palette as CSS custom properties.
     */
    function remotePaletteStyle(palette) {
      return {
        "--remote-terminal-fg": palette.foreground,
        "--remote-terminal-bg": palette.background,
        "--remote-terminal-cursor": palette.cursor,
        "--remote-terminal-muted": palette.muted,
        "--remote-terminal-primary": palette.primary,
      };
    }

    /**
     * Builds a style descriptor for one terminal cell.
     */
    function terminalCellStyle(cell) {
      const palette = activeTerminalPalette();
      const fg = cell.fg || palette.foreground;
      const bg = cell.bg || palette.background;
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
      const palette = activeTerminalPalette();
      const fg = run.fg || run.color || palette.foreground;
      const bg = run.bg || run.backgroundColor || palette.background;
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
      const palette = activeTerminalPalette();
      const fg = run.fg || run.color || palette.foreground;
      const bg = run.bg || run.backgroundColor || palette.background;
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
     * Replaces only changed terminal row nodes while preserving scroll gestures.
     * Example: two changed rows in a 24-row TUI -> two DOM replacements.
     */
    function patchTerminalRows(patches, expectedRowCount) {
      const output = terminalOutputElement.value;
      if (!output || output.childElementCount !== expectedRowCount) {
        renderTerminalRows(activeTab()?.rows ?? []);
        return;
      }

      const replacements = [];
      for (const patch of patches) {
        const current = output.children.item(patch.index);
        if (!current) {
          renderTerminalRows(activeTab()?.rows ?? []);
          return;
        }
        replacements.push([current, terminalRowNode(patch.row)]);
      }
      // 浏览器会把连续 replaceWith 合并到同一 paint，避免整棵 DOM 失效。
      for (const [current, replacement] of replacements) {
        current.replaceWith(replacement);
      }
    }

    /**
     * Appends protocol-guaranteed new rows and returns removed DOM height.
     * Example: 2 new rows and no trim -> append 2 nodes and return 0.
     */
    function appendTerminalRows(rows, removedRowCount, previousRowCount) {
      const output = terminalOutputElement.value;
      if (!output || output.childElementCount !== previousRowCount) {
        return null;
      }

      // 触发条件：browser buffer 达到 500 行后仍收到 append。
      // 不能替换整棵 DOM：移动端正在进行的原生滚动手势会被浏览器取消。
      // 防止回归：持续输出时仍可稳定上划查看历史内容。
      const removedExistingRows = Math.min(
        removedRowCount,
        previousRowCount,
      );
      let removedHeight = 0;
      for (let index = 0; index < removedExistingRows; index += 1) {
        const firstRow = output.firstElementChild;
        if (firstRow) {
          removedHeight += firstRow.getBoundingClientRect().height;
          firstRow.remove();
        }
      }
      const removedNewRows = Math.max(0, removedRowCount - previousRowCount);
      const fragment = document.createDocumentFragment();
      for (const row of rows.slice(removedNewRows)) {
        fragment.appendChild(terminalRowNode(row));
      }
      output.appendChild(fragment);
      return removedHeight;
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
     * Renders the currently active tab into the shared terminal output node.
     */
    function renderActiveTerminal() {
      refreshTabs();
    }

    /**
     * Activates one open browser tab without opening duplicate sockets.
     */
    function activateAgentTab(tabId) {
      keyMenuOpen.value = false;
      handleTerminalScroll();
      cancelTerminalScroll();
      activeTabId.value = tabId;
      const tab = activeTab();
      nextTick(async () => {
        renderActiveTerminal();
        sessionVirtualizer.value.measure();
        await renderSessionMermaid(terminalOutputElement.value);
        if (terminalShouldFollowOutput(tab)) {
          scrollTerminalToBottom(tab);
        } else {
          restoreTerminalScroll(tab);
        }
        if (draftInputElement.value && tab) {
          draftInputElement.value.value = tab.draft;
        }
      });
    }

    /**
     * Groups consecutive tool events into one compact render row.
     *
     * Example: tool + result + tool + result -> one collapsed Tools (4) row.
     */
    function sessionRenderRows(events, agentBusy) {
      const rows = [];
      let tools = [];
      // Flush only at a non-tool boundary so one agent operation stays compact.
      // Example: two buffered tool events -> one tool_group row.
      const flushTools = (mayBeRunning) => {
        const resultKeys = new Set(
          tools
            .filter((event) => event.kind === "tool_result")
            .map((event) => event.key),
        );
        const renderedTools = tools.map((event) => ({
          ...event,
          running:
            mayBeRunning &&
            event.kind === "tool" &&
            !resultKeys.has(event.key),
        }));
        const running = renderedTools.some((event) => event.running);
        if (tools.length === 1) {
          rows.push(renderedTools[0]);
        } else if (tools.length > 1) {
          rows.push({
            kind: "tool_group",
            id: `tool-group:${tools[0].id}:${tools[tools.length - 1].id}`,
            events: renderedTools,
            running,
          });
        }
        tools = [];
      };
      for (const event of events) {
        if (event.kind === "tool" || event.kind === "tool_result") {
          tools.push(event);
          continue;
        }
        flushTools(false);
        rows.push(event);
      }
      // Only a trailing tool block can belong to the currently running turn.
      // Historical blocks may miss results after an abort and must stay closed.
      flushTools(agentBusy);
      return rows;
    }

    /**
     * Closes one browser tab and selects a neighboring tab when needed.
     */
    function closeAgentTab(tab, event) {
      keyMenuOpen.value = false;
      event?.stopPropagation();
      const index = tabs.value.findIndex((candidate) => candidate.id === tab.id);
      if (index < 0) {
        return;
      }
      closeAgentSocket(tab, true);
      const nextTabs = tabs.value.filter((candidate) => candidate.id !== tab.id);
      tabs.value = nextTabs;
      if (activeTabId.value === tab.id) {
        const next = nextTabs[Math.max(0, index - 1)] ?? nextTabs[index] ?? null;
        activeTabId.value = next?.id ?? null;
        nextTick(() => {
          const active = activeTab();
          renderActiveTerminal();
          if (terminalShouldFollowOutput(active)) {
            scrollTerminalToBottom(active);
          } else {
            restoreTerminalScroll(active);
          }
        });
      }
    }

    /**
     * Renders one normalized session event with native selectable text.
     *
     * Example: tool result -> collapsed details row; assistant -> message row.
     */
    function renderSessionEvent(event) {
      if (event.kind === "history_loader") {
        return (
          <div class="remote-shell__session-history-loader" key={event.id}>
            <button
              type="button"
              class={[
                "remote-shell__session-history-button",
                event.error
                  ? "remote-shell__session-history-button--error"
                  : "",
              ]}
              aria-label="Load 100 earlier records"
              title={event.error || "Load 100 earlier records"}
              disabled={event.loading}
              onClick={() => loadEarlierSession()}
            >
              {event.loading ? (
                <span
                  class="remote-shell__session-history-spinner"
                  aria-hidden="true"
                />
              ) : (
                <span aria-hidden="true">↑</span>
              )}
            </button>
          </div>
        );
      }
      if (event.kind === "tool_group") {
        return (
          <details
            class="remote-shell__session-tool-group"
            key={event.id}
            open={event.running}
          >
            <summary>Tools ({event.events.length})</summary>
            <div class="remote-shell__session-tool-group-items">
              {event.events.map(renderSessionEvent)}
            </div>
          </details>
        );
      }
      if (event.kind === "tool" || event.kind === "tool_result") {
        return (
          <details
            class={[
              "remote-shell__session-tool",
              `remote-shell__session-tool--${event.kind}`,
            ]}
            key={event.id}
            open={event.running}
          >
            <summary>{event.title}</summary>
            <pre>{event.text}</pre>
          </details>
        );
      }
      if (event.kind === "reasoning") {
        return (
          <details class="remote-shell__session-reasoning" key={event.id}>
            <summary>{event.title}</summary>
            <div
              class="remote-shell__session-text remote-shell__session-markdown"
              innerHTML={event.html}
            />
          </details>
        );
      }
      if (event.kind === "compacted") {
        return (
          <div class="remote-shell__session-compacted" key={event.id}>
            <span>{event.text}</span>
          </div>
        );
      }
      return (
        <article
          class={[
            "remote-shell__session-item",
            `remote-shell__session-item--${event.kind}`,
          ]}
          key={event.id}
        >
          <div class="remote-shell__session-content">
            <div class="remote-shell__session-label">{event.title}</div>
            <div
              class="remote-shell__session-text remote-shell__session-markdown"
              innerHTML={event.html}
            />
          </div>
        </article>
      );
    }

    /**
     * Measures one mounted session row for variable-height virtualization.
     *
     * Example: expanded Tool row -> update its cached height.
     */
    function measureSessionRow(element) {
      if (element) {
        sessionVirtualizer.value.measureElement(element);
      }
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
      return activeTabId.value === agentTabId(workspace, row, col, agent);
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
          <span
            class={[
              "remote-shell__agent-state-dot",
              `remote-shell__agent-state-dot--${agentDisplayActivity(agent)}`,
            ]}
            aria-hidden="true"
          />
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
      closeWorkspaceSocket(true);
      closeAllAgentSockets();
      cancelTerminalScroll();
      disconnectVisualViewportObserver();
    }

    onMounted(() => {
      observeVisualViewportSize();
      loadWorkspaceTree();
      connectWorkspaceSocket();
      renderActiveTerminal();
    });
    onBeforeUnmount(cleanupRemoteShell);

    /**
     * Renders the remote shell with drawer, terminal, and composer.
     */
    return () => {
      const tab = activeTab();
      const binding = tab?.binding ?? null;
      const terminalBanner = tab?.error || (tab?.status === "dis" ? "dis" : "");
      const palette = activeTerminalPalette();
      return (
      <div
        class={{
          "remote-shell": true,
          "remote-shell--spotlight": spotlightMode.value,
          "remote-shell--spotlight-collapsed":
            spotlightMode.value && spotlightSidebarCollapsed.value,
        }}
        style={remotePaletteStyle(palette)}
      >
        {spotlightMode.value ? (
          <aside class="remote-shell__spotlight-sidebar">
            <div class="remote-shell__spotlight-toolbar">
              {!spotlightSidebarCollapsed.value ? (
                <button
                  class="remote-shell__spotlight-exit"
                  type="button"
                  aria-label="退出专注模式"
                  title="退出专注模式"
                  onClick={exitSpotlightMode}
                >
                  <span aria-hidden="true" />
                </button>
              ) : null}
              <button
                class={{
                  "remote-shell__spotlight-collapse": true,
                  "remote-shell__spotlight-collapse--collapsed":
                    spotlightSidebarCollapsed.value,
                }}
                type="button"
                aria-label={spotlightSidebarCollapsed.value ? "展开专注侧栏" : "收起专注侧栏"}
                title={spotlightSidebarCollapsed.value ? "展开专注侧栏" : "收起专注侧栏"}
                onClick={() => {
                  spotlightSidebarCollapsed.value = !spotlightSidebarCollapsed.value;
                }}
              >
                <span aria-hidden="true" />
              </button>
              {!spotlightSidebarCollapsed.value ? (
                <label
                  class="remote-shell__spotlight-rank-mode"
                  title="等级模式"
                >
                  <input
                    type="checkbox"
                    aria-label="等级模式"
                    checked={spotlightRankMode.value}
                    onChange={updateSpotlightRankMode}
                  />
                </label>
              ) : null}
            </div>
            <nav class="remote-shell__spotlight-list" aria-label="Spotlight agents">
              {spotlightAgentBindings().map((item) => (
                <button
                  class={{
                    "remote-shell__spotlight-agent": true,
                    "remote-shell__spotlight-agent--active":
                      activeTabId.value === item.id,
                  }}
                  type="button"
                  key={`${item.workspace.workspace_id}:${item.agent.agent_id}`}
                  title={`${item.agent.title}\n${item.workspace.name}`}
                  aria-label={`${item.agent.title} / ${item.workspace.name}`}
                  onClick={() =>
                    selectAgent(item.workspace, item.row, item.col, item.agent)
                  }
                >
                  <span class="remote-shell__spotlight-star" aria-hidden="true">★</span>
                  <span class="remote-shell__spotlight-copy">
                    <span class="remote-shell__spotlight-title">{item.agent.title}</span>
                    <span class="remote-shell__spotlight-separator" aria-hidden="true">·</span>
                    <span class="remote-shell__spotlight-workspace">
                      {spotlightWorkspaceLeaf(item.workspace.name)}
                    </span>
                  </span>
                  <span
                    class={[
                      "remote-shell__spotlight-status",
                      `remote-shell__spotlight-status--${agentDisplayActivity(item.agent)}`,
                    ]}
                    aria-label={agentDisplayActivity(item.agent)}
                  />
                </button>
              ))}
            </nav>
          </aside>
        ) : null}

        {!spotlightMode.value ? <header class="remote-shell__header">
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
            <div class="remote-shell__title">GSDV</div>
            <span class="remote-shell__heading-divider" aria-hidden="true" />
            <div class="remote-shell__subtitle">
              {binding
                ? `${binding.workspace.name} / ${binding.agent.title}`
                : "No agent selected"}
            </div>
          </div>
        </header> : null}

        {!spotlightMode.value && tabs.value.length > 0 ? (
          <div class="remote-shell__tabs" aria-label="Open agents">
            {tabs.value.map((item) => (
              <button
                class={{
                  "remote-shell__tab": true,
                  "remote-shell__tab--active": item.id === activeTabId.value,
                  "remote-shell__tab--disconnected": item.status === "dis",
                }}
                type="button"
                key={item.id}
                onClick={() => activateAgentTab(item.id)}
              >
                <span
                  class={[
                    "remote-shell__agent-state-dot",
                    `remote-shell__agent-state-dot--${tabDisplayActivity(item)}`,
                  ]}
                  aria-hidden="true"
                />
                <span class="remote-shell__tab-label">{item.title}</span>
                <span class="remote-shell__tab-state">
                  {item.status === "dis"
                    ? "dis"
                    : tabDisplayActivity(item)}
                </span>
                <span
                  class="remote-shell__tab-close"
                  role="button"
                  aria-label={`Close ${item.title}`}
                  onClick={(event) => closeAgentTab(item, event)}
                >
                  x
                </span>
              </button>
            ))}
          </div>
        ) : null}

        <main class="remote-shell__terminal" aria-label="Agent session output">
          <div
            class="remote-shell__terminal-output remote-shell__session"
            ref={terminalOutputElement}
            onScroll={handleTerminalScroll}
          >
            {tab?.initialLoading ? (
              <div class="remote-shell__terminal-empty">{tab.status}</div>
            ) : sessionRows.value.length ? (
              <div
                class="remote-shell__session-virtual-space"
                style={{
                  height: `${sessionVirtualizer.value.getTotalSize()}px`,
                }}
              >
                {visibleSessionRows.value.map((virtualRow) => (
                  <div
                    class="remote-shell__session-virtual-row"
                    data-index={virtualRow.index}
                    data-row-key={
                      sessionRows.value[virtualRow.index]?.id ?? virtualRow.key
                    }
                    key={virtualRow.key}
                    ref={measureSessionRow}
                    style={{
                      transform: `translateY(${virtualRow.start}px)`,
                    }}
                  >
                    {renderSessionEvent(sessionRows.value[virtualRow.index])}
                  </div>
                ))}
              </div>
            ) : (
              <div class="remote-shell__terminal-empty">
                {tab?.status ?? "Select an agent from the workspace drawer."}
              </div>
            )}
          </div>
          {terminalBanner ? (
            <div class="remote-shell__terminal-banner">
              {terminalBanner}
            </div>
          ) : null}
        </main>

        {binding ? (
          <div
            class={[
              "remote-shell__activity-strip",
              `remote-shell__activity-strip--${tabDisplayActivity(tab)}`,
            ]}
            role="status"
            aria-live="polite"
          >
            <span
              class={[
                "remote-shell__agent-state-dot",
                `remote-shell__agent-state-dot--${tabDisplayActivity(tab)}`,
              ]}
              aria-hidden="true"
            />
            {tabDisplayActivity(tab) === "busy" ? (
              <>
                <span>Agent is responding</span>
                <span class="remote-shell__activity-dots" aria-hidden="true">
                  <i />
                  <i />
                  <i />
                </span>
              </>
            ) : (
              <span>{tabDisplayActivity(tab)}</span>
            )}
          </div>
        ) : null}

        <footer class="remote-shell__composer">
          <div class="remote-shell__key-control">
            <Button
              class="remote-shell__action-button"
              aria-label="Terminal keys"
              aria-expanded={keyMenuOpen.value}
              disabled={!binding}
              onClick={toggleKeyMenu}
            >
              ESC
            </Button>
            {keyMenuOpen.value ? (
              <div class="remote-shell__key-menu" role="menu" aria-label="Terminal keys">
                {[
                  ["up", "↑", "Up"],
                  ["down", "↓", "Down"],
                  ["left", "←", "Left"],
                  ["right", "→", "Right"],
                  ["enter", "Enter", "Enter"],
                  ["esc", "Esc", "Escape"],
                ].map(([key, label, ariaLabel]) => (
                  <button
                    class="remote-shell__key-menu-button"
                    type="button"
                    role="menuitem"
                    aria-label={ariaLabel}
                    key={key}
                    onClick={() => sendAgentKey(key)}
                  >
                    {label}
                  </button>
                ))}
              </div>
            ) : null}
          </div>
          <Button
            class="remote-shell__action-button"
            aria-label="Select image"
            disabled={!binding}
            onClick={openImagePicker}
          >
            IMG
          </Button>
          <textarea
            class="remote-shell__input"
            aria-label="Agent input"
            ref={draftInputElement}
            disabled={!binding}
            placeholder="Send input to the selected agent"
            rows="1"
            value={tab?.draft ?? ""}
            onInput={handleDraftInput}
            onFocus={scheduleVisualViewportSync}
            onBlur={scheduleVisualViewportSync}
            onKeydown={handleDraftKeydown}
          />
          <Button
            class="remote-shell__send-button"
            aria-label="Send input"
            type="primary"
            disabled={!binding}
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
          rootClassName="remote-shell__drawer"
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
          <div class="remote-shell__drawer-footer">
            <Button block onClick={enterSpotlightMode}>
              ★ 专注模式
            </Button>
          </div>
        </Drawer>
      </div>
      );
    };
  },
});
