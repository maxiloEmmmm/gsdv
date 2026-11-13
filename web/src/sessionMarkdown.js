import MarkdownIt from "markdown-it";

const markdown = new MarkdownIt({
  html: false,
  linkify: true,
  typographer: false,
  breaks: false,
});

markdown.renderer.rules.link_open = (tokens, index, options, environment, renderer) => {
  tokens[index].attrSet("target", "_blank");
  tokens[index].attrSet("rel", "noopener noreferrer");
  return renderer.renderToken(tokens, index, options, environment);
};

/**
 * Renders trusted parser output while escaping raw HTML from session files.
 *
 * Example: "**done**" -> "<p><strong>done</strong></p>".
 */
export function renderSessionMarkdown(text) {
  return markdown.render(typeof text === "string" ? text : "");
}

/**
 * Caches Markdown HTML only for message-like session events.
 *
 * Example: assistant event -> event with html; tool result -> unchanged event.
 */
export function withSessionMarkdown(event) {
  if (!["user", "assistant", "reasoning"].includes(event.kind)) {
    return event;
  }
  const html = renderSessionMarkdown(event.text);
  if (!html.trim()) {
    return null;
  }
  return { ...event, html };
}
