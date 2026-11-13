/**
 * Converts an unknown content value into readable session text.
 *
 * Example: [{ type: "text", text: "ok" }] -> "ok".
 */
function contentText(content) {
  if (typeof content === "string") {
    return content;
  }
  if (!Array.isArray(content)) {
    return "";
  }
  return content
    .map((item) => {
      if (typeof item === "string") {
        return item;
      }
      if (!item || typeof item !== "object") {
        return "";
      }
      return item.text ?? item.input_text ?? item.output_text ?? "";
    })
    .filter(Boolean)
    .join("\n");
}

/**
 * Formats structured tool arguments without changing string arguments.
 *
 * Example: { path: "/tmp/a" } -> a two-line JSON string.
 */
function toolText(value) {
  if (typeof value === "string") {
    return value;
  }
  if (value === undefined || value === null) {
    return "";
  }
  try {
    return JSON.stringify(value, null, 2);
  } catch (_error) {
    return String(value);
  }
}

/**
 * Builds one normalized browser event when it has visible content.
 *
 * Example: assistant + "done" -> one renderable assistant event.
 */
function visibleEvent(kind, text, title, key) {
  const normalized = typeof text === "string" ? text.trimEnd() : "";
  if (!normalized) {
    return null;
  }
  return { kind, text: normalized, title, key };
}

/**
 * Detects Codex startup context that is persisted with a user role.
 *
 * Example: "# AGENTS.md instructions\n..." -> hidden browser event.
 */
function isCodexStartupContext(payload) {
  if (payload?.role !== "user") {
    return false;
  }
  const text = contentText(payload.content).trimStart();
  return (
    text.startsWith("# AGENTS.md instructions") ||
    text.startsWith("<environment_context>")
  );
}

/**
 * Parses one Codex rollout JSONL record into browser events.
 *
 * Example: response_item/message/assistant -> assistant event.
 */
function codexEvents(record, fallbackKey) {
  if (!record || typeof record !== "object") {
    return [];
  }
  if (record.type === "compacted") {
    return [
      {
        kind: "compacted",
        text: "Context compacted",
        title: "Context",
        key: record.id ?? fallbackKey,
      },
    ];
  }
  if (record.type !== "response_item") {
    return [];
  }

  const payload = record.payload;
  if (!payload || typeof payload !== "object") {
    return [];
  }
  const baseKey = payload.id ?? payload.call_id ?? fallbackKey;
  if (payload.type === "message") {
    if (payload.role !== "user" && payload.role !== "assistant") {
      return [];
    }
    // Codex records repository instructions as a user message at session start.
    // Rendering it exposes internal context and makes /clear look ineffective.
    if (isCodexStartupContext(payload)) {
      return [];
    }
    const event = visibleEvent(
      payload.role,
      contentText(payload.content),
      payload.role === "user" ? "You" : "Agent",
      baseKey,
    );
    return event ? [event] : [];
  }
  if (payload.type === "reasoning") {
    const text =
      contentText(payload.summary) ||
      contentText(payload.content) ||
      payload.text ||
      "";
    const event = visibleEvent("reasoning", text, "Reasoning", baseKey);
    return event ? [event] : [];
  }
  if (payload.type === "function_call" || payload.type === "custom_tool_call") {
    const event = visibleEvent(
      "tool",
      toolText(payload.arguments ?? payload.input),
      payload.name || "Tool call",
      baseKey,
    );
    return event ? [event] : [];
  }
  if (
    payload.type === "function_call_output" ||
    payload.type === "custom_tool_call_output"
  ) {
    const event = visibleEvent(
      "tool_result",
      toolText(payload.output),
      "Tool result",
      baseKey,
    );
    return event ? [event] : [];
  }
  return [];
}

/**
 * Parses one Claude content block into a normalized browser event.
 *
 * Example: tool_use block -> expandable tool event.
 */
function claudeBlockEvent(block, role, baseKey, index) {
  if (typeof block === "string") {
    return visibleEvent(
      role,
      block,
      role === "user" ? "You" : "Agent",
      `${baseKey}:${index}`,
    );
  }
  if (!block || typeof block !== "object") {
    return null;
  }
  const key = block.id ?? block.tool_use_id ?? `${baseKey}:${index}`;
  if (block.type === "text") {
    return visibleEvent(
      role,
      block.text,
      role === "user" ? "You" : "Agent",
      key,
    );
  }
  if (block.type === "thinking") {
    return visibleEvent("reasoning", block.thinking, "Reasoning", key);
  }
  if (block.type === "tool_use") {
    return visibleEvent("tool", toolText(block.input), block.name || "Tool call", key);
  }
  if (block.type === "tool_result") {
    return visibleEvent(
      "tool_result",
      contentText(block.content) || toolText(block.content),
      block.is_error ? "Tool error" : "Tool result",
      key,
    );
  }
  return null;
}

/**
 * Parses one Claude project JSONL record into browser events.
 *
 * Example: assistant message with text and tool_use -> two events.
 */
function claudeEvents(record, fallbackKey) {
  if (!record || (record.type !== "user" && record.type !== "assistant")) {
    return [];
  }
  const message = record.message;
  if (!message || typeof message !== "object") {
    return [];
  }
  const role = message.role === "user" ? "user" : "assistant";
  const baseKey = record.uuid ?? message.id ?? fallbackKey;
  const content = Array.isArray(message.content)
    ? message.content
    : [message.content];
  return content
    .map((block, index) => claudeBlockEvent(block, role, baseKey, index))
    .filter(Boolean);
}

/**
 * Derives the latest runtime activity carried by one persisted session record.
 *
 * Example: Codex task_started -> busy; Claude end_turn -> idle.
 */
export function sessionActivity(agentKind, record) {
  if (!record || typeof record !== "object") {
    return null;
  }
  if (agentKind === "claude") {
    if (record.type === "user") {
      return "busy";
    }
    if (record.type !== "assistant") {
      return null;
    }
    const stopReason = record.message?.stop_reason;
    return stopReason === "tool_use" ? "busy" : "idle";
  }
  if (record.type !== "event_msg" || !record.payload) {
    return null;
  }
  const eventType = record.payload.type;
  if (eventType === "task_started") {
    return "busy";
  }
  if (eventType === "task_complete" || eventType === "turn_aborted") {
    return "idle";
  }
  return null;
}

/**
 * Parses one provider JSONL record without retaining the raw record.
 *
 * Example: provider "codex" with token_count -> no visible events.
 */
export function sessionEvents(agentKind, record, fallbackKey) {
  return agentKind === "claude"
    ? claudeEvents(record, fallbackKey)
    : codexEvents(record, fallbackKey);
}
