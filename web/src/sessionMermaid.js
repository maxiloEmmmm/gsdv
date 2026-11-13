let mermaidPromise = null;
let diagramSerial = 0;

/**
 * Loads and configures Mermaid only when a rendered message needs it.
 *
 * Example: first Mermaid block -> dynamic import; later blocks -> cached API.
 */
async function loadMermaid() {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then(({ default: mermaid }) => {
      mermaid.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: "dark",
        fontFamily: "Inter, ui-sans-serif, system-ui, sans-serif",
        themeVariables: {
          background: "#0b0d10",
          primaryColor: "#171b20",
          primaryTextColor: "#e5e7eb",
          primaryBorderColor: "#4b5563",
          lineColor: "#94a3b8",
          secondaryColor: "#111827",
          tertiaryColor: "#1f2937",
        },
      });
      return mermaid;
    });
  }
  return mermaidPromise;
}

/**
 * Replaces newly mounted Mermaid code fences with strict-mode SVG diagrams.
 *
 * Example: `pre > code.language-mermaid` -> `.remote-shell__mermaid > svg`.
 */
export async function renderSessionMermaid(root) {
  if (!root) {
    return false;
  }
  const blocks = Array.from(
    root.querySelectorAll("pre > code.language-mermaid"),
  );
  if (blocks.length === 0) {
    return false;
  }

  const mermaid = await loadMermaid();
  let rendered = false;
  for (const block of blocks) {
    const source = block.textContent ?? "";
    const container = block.parentElement;
    if (!container || container.dataset.mermaidState) {
      continue;
    }
    container.dataset.mermaidState = "loading";
    try {
      const id = `gsdv-mermaid-${diagramSerial}`;
      diagramSerial += 1;
      const result = await mermaid.render(id, source);
      if (!container.isConnected) {
        continue;
      }
      const diagram = document.createElement("div");
      diagram.className = "remote-shell__mermaid";
      diagram.innerHTML = result.svg;
      container.replaceWith(diagram);
      result.bindFunctions?.(diagram);
      rendered = true;
    } catch (error) {
      container.dataset.mermaidState = "error";
      container.classList.add("remote-shell__mermaid-error");
      container.title = error instanceof Error ? error.message : String(error);
    }
  }
  return rendered;
}
