import type { Mermaid, MermaidConfig, RenderResult } from "mermaid";
import { sanitizeMermaidSvg } from "./markdown";

export type MermaidTheme = "default" | "dark";

export interface MermaidRenderOutcome {
  errors: Error[];
  rendered: number;
  status: "complete" | "stale";
}

type MermaidEngine = Pick<Mermaid, "initialize" | "render">;

interface MermaidRenderOptions {
  idPrefix: string;
  isCurrent: () => boolean;
  loadEngine?: () => Promise<MermaidEngine>;
  theme: MermaidTheme;
}

const MERMAID_CODE_SELECTOR = "pre > code.language-mermaid";
const MAX_MERMAID_TEXT_SIZE = 50_000;
const MAX_MERMAID_EDGES = 500;
const SECURE_MERMAID_CONFIG_KEYS = [
  "secure",
  "securityLevel",
  "startOnLoad",
  "maxTextSize",
  "maxEdges",
  "suppressErrorRendering",
  "theme",
  "themeVariables",
  "themeCSS",
  "darkMode",
  "htmlLabels",
  "fontFamily",
  "altFontFamily",
  "dompurifyConfig",
  "logLevel",
  "arrowMarkerAbsolute",
  "deterministicIds",
  "deterministicIDSeed",
  "handDrawnSeed",
] as const;

let mermaidRenderQueue: Promise<void> = Promise.resolve();

async function loadMermaid(): Promise<MermaidEngine> {
  return (await import("mermaid")).default;
}

function renderConfiguration(theme: MermaidTheme, seed: string): MermaidConfig {
  return {
    darkMode: theme === "dark",
    deterministicIDSeed: seed,
    deterministicIds: true,
    handDrawnSeed: 1,
    htmlLabels: false,
    logLevel: "fatal",
    maxEdges: MAX_MERMAID_EDGES,
    maxTextSize: MAX_MERMAID_TEXT_SIZE,
    secure: [...SECURE_MERMAID_CONFIG_KEYS],
    securityLevel: "strict",
    startOnLoad: false,
    suppressErrorRendering: true,
    theme,
  };
}

function errorFrom(reason: unknown): Error {
  return reason instanceof Error ? reason : new Error(String(reason));
}

function markFailedCodeBlock(code: HTMLElement): void {
  const pre = code.parentElement;
  if (!(pre instanceof HTMLPreElement)) return;
  pre.dataset.mermaidState = "error";

  const message = document.createElement("p");
  message.className = "mermaid-error-message";
  message.setAttribute("role", "status");
  message.textContent = "Unable to render this Mermaid diagram. Its source is shown above.";
  pre.insertAdjacentElement("afterend", message);
}

function replaceCodeBlock(code: HTMLElement, result: RenderResult): void {
  const pre = code.parentElement;
  if (!(pre instanceof HTMLPreElement)) return;

  const svg = sanitizeMermaidSvg(result.svg);
  if (!svg.trim()) throw new Error("Mermaid returned an empty SVG after sanitization.");

  const figure = document.createElement("figure");
  figure.className = "mermaid-diagram";
  figure.innerHTML = svg;
  pre.replaceWith(figure);
}

async function renderCodeBlocks(
  codeBlocks: HTMLElement[],
  options: MermaidRenderOptions,
): Promise<MermaidRenderOutcome> {
  const errors: Error[] = [];
  let rendered = 0;
  let engine: MermaidEngine;

  try {
    engine = await (options.loadEngine ?? loadMermaid)();
  } catch (reason) {
    const error = errorFrom(reason);
    errors.push(error);
    for (const code of codeBlocks) markFailedCodeBlock(code);
    return { errors, rendered, status: "complete" };
  }

  for (const [index, code] of codeBlocks.entries()) {
    if (!options.isCurrent()) return { errors, rendered, status: "stale" };

    const id = `${options.idPrefix}-${index + 1}`;
    const definition = code.textContent ?? "";
    code.parentElement?.setAttribute("data-mermaid-state", "rendering");

    try {
      engine.initialize(renderConfiguration(options.theme, id));
      const result = await engine.render(id, definition);
      if (!options.isCurrent() || !code.isConnected) {
        return { errors, rendered, status: "stale" };
      }
      replaceCodeBlock(code, result);
      rendered += 1;
    } catch (reason) {
      const error = errorFrom(reason);
      errors.push(error);
      if (options.isCurrent() && code.isConnected) markFailedCodeBlock(code);
    }
  }

  return { errors, rendered, status: "complete" };
}

/**
 * Replaces settled `mermaid` code fences with sanitized SVG diagrams.
 *
 * Rendering batches share a queue because Mermaid configuration is process-global.
 */
export function renderMermaidCodeBlocks(
  container: HTMLElement,
  options: MermaidRenderOptions,
): Promise<MermaidRenderOutcome> {
  const codeBlocks = Array.from(container.querySelectorAll<HTMLElement>(MERMAID_CODE_SELECTOR));
  if (codeBlocks.length === 0) {
    return Promise.resolve({ errors: [], rendered: 0, status: "complete" });
  }

  const task = mermaidRenderQueue.then(() => renderCodeBlocks(codeBlocks, options));
  mermaidRenderQueue = task.then(
    () => undefined,
    () => undefined,
  );
  return task;
}
