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
 * Parses tool arguments that providers may persist as JSON strings.
 *
 * Example: '{"questions":[]}' -> { questions: [] }.
 */
function toolInput(value) {
  if (typeof value !== "string") {
    return value;
  }
  try {
    return JSON.parse(value);
  } catch (_error) {
    return null;
  }
}

/**
 * Normalizes one provider question without retaining unrelated tool fields.
 *
 * Example: AskUserQuestion option objects -> browser question data.
 */
function questionnaireQuestion(question, index) {
  if (!question || typeof question !== "object") {
    return null;
  }
  const prompt = String(question.question ?? "").trim();
  if (!prompt) {
    return null;
  }
  const options = Array.isArray(question.options)
    ? question.options
        .filter((option) => option && typeof option === "object")
        .map((option) => ({
          label: String(option.label ?? "").trim(),
          description: String(option.description ?? "").trim(),
          preview: String(option.preview ?? "").trim(),
        }))
        .filter((option) => option.label)
    : [];
  return {
    id: String(question.id ?? `question-${index}`),
    header: String(question.header ?? `Q${index + 1}`).trim(),
    question: prompt,
    multiSelect: question.multiSelect === true,
    options,
  };
}

/**
 * Builds an interactive event for Claude AskUserQuestion or Codex input requests.
 *
 * Example: two questions -> one questionnaire event with two tabs.
 */
function questionnaireEvent(name, input, key) {
  if (name !== "AskUserQuestion" && name !== "request_user_input") {
    return null;
  }
  const parsed = toolInput(input);
  const questions = Array.isArray(parsed?.questions)
    ? parsed.questions
        .map(questionnaireQuestion)
        .filter(Boolean)
    : [];
  if (questions.length === 0) {
    return null;
  }
  return {
    kind: "questionnaire",
    title: "Questions",
    key,
    questions,
  };
}

/**
 * Detects a completed Claude questionnaire result for historical locking.
 *
 * Example: AskUserQuestion result text -> questionnaire_result event.
 */
function claudeQuestionnaireResult(block, key) {
  const answered =
    typeof block?.content === "string" &&
    block.content.startsWith("Your questions have been answered:");
  const cancelled =
    typeof block?.content === "string" &&
    block.content.startsWith("The user doesn't want to proceed with this tool use.");
  if (
    block?.type !== "tool_result" ||
    (!answered && !cancelled)
  ) {
    return null;
  }
  return {
    kind: "questionnaire_result",
    title: "Answers submitted",
    text: block.content,
    key,
    cancelled,
  };
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
    const toolKey = payload.call_id ?? baseKey;
    const questionnaire = questionnaireEvent(
      payload.name,
      payload.arguments ?? payload.input,
      toolKey,
    );
    if (questionnaire) {
      return [questionnaire];
    }
    const event = visibleEvent(
      "tool",
      toolText(payload.arguments ?? payload.input),
      payload.name || "Tool call",
      toolKey,
    );
    return event ? [event] : [];
  }
  if (
    payload.type === "function_call_output" ||
    payload.type === "custom_tool_call_output"
  ) {
    const toolKey = payload.call_id ?? baseKey;
    const event = visibleEvent(
      "tool_result",
      toolText(payload.output),
      "Tool result",
      toolKey,
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
    const questionnaire = questionnaireEvent(block.name, block.input, key);
    if (questionnaire) {
      return questionnaire;
    }
    return visibleEvent("tool", toolText(block.input), block.name || "Tool call", key);
  }
  if (block.type === "tool_result") {
    const questionnaireResult = claudeQuestionnaireResult(block, key);
    if (questionnaireResult) {
      return questionnaireResult;
    }
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
 * Parses Claude's XML-like internal user records without styling them as input.
 *
 * Example: <task-notification>...</task-notification> -> collapsed notification.
 */
function claudeInternalUserEvent(content, baseKey) {
  if (typeof content !== "string") {
    return undefined;
  }
  const trimmed = content.trimStart();
  const isTaskNotification = trimmed.startsWith("<task-notification>");
  const isCommand = trimmed.startsWith("<command-name>");
  const isCommandOutput = trimmed.startsWith("<local-command-stdout>");
  if (trimmed.startsWith("<local-command-caveat>")) {
    return null;
  }
  if (!isTaskNotification && !isCommand && !isCommandOutput) {
    return undefined;
  }

  const document = new DOMParser().parseFromString(
    `<gsdv-record>${content}</gsdv-record>`,
    "application/xml",
  );
  const elementText = (name) =>
    document.querySelector(name)?.textContent?.trim() ?? "";
  if (isTaskNotification) {
    const summary = elementText("summary");
    const taskId = elementText("task-id");
    const status = elementText("status");
    const text = [
      taskId ? `Task: ${taskId}` : "",
      status ? `Status: ${status}` : "",
      elementText("event"),
      elementText("result"),
      elementText("note"),
    ]
      .filter(Boolean)
      .join("\n\n");
    return visibleEvent(
      "notification",
      text || content,
      summary || "Task notification",
      baseKey,
    );
  }
  if (isCommand) {
    const name = elementText("command-name") || "Local command";
    const text = elementText("command-args") || elementText("command-message");
    return visibleEvent(
      "notification",
      text || name,
      `Local command · ${name}`,
      baseKey,
    );
  }
  return visibleEvent(
    "notification",
    elementText("local-command-stdout") || content,
    "Local command output",
    baseKey,
  );
}

/**
 * Parses one Claude project JSONL record into browser events.
 *
 * Example: assistant message with text and tool_use -> two events.
 */
function claudeEvents(record, fallbackKey) {
  if (!record || typeof record !== "object") {
    return [];
  }
  if (record.type === "queue-operation") {
    if (record.operation !== "enqueue") {
      return [];
    }
    const baseKey = record.timestamp ?? fallbackKey;
    const internalEvent = claudeInternalUserEvent(record.content ?? "", baseKey);
    if (internalEvent !== undefined) {
      return internalEvent ? [internalEvent] : [];
    }
    const event = visibleEvent(
      "user",
      record.content,
      "You",
      baseKey,
    );
    return event ? [event] : [];
  }
  if (record.type !== "user" && record.type !== "assistant") {
    return [];
  }
  const message = record.message;
  if (!message || typeof message !== "object") {
    return [];
  }
  const role = message.role === "user" ? "user" : "assistant";
  const baseKey = record.uuid ?? message.id ?? fallbackKey;
  if (role === "user" && typeof message.content === "string") {
    const internalEvent = claudeInternalUserEvent(message.content, baseKey);
    if (internalEvent !== undefined) {
      return internalEvent ? [internalEvent] : [];
    }
  }
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
