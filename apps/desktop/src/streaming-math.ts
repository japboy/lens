import { markdownInline } from "@generative-dom/plugin-markdown-inline";
import { markdownLink } from "@generative-dom/plugin-markdown-link";
import { readMathSpan } from "./markdown-math";

const inline = markdownInline();
const links = markdownLink();

// Containers remain in the source until upstream recursively parses their bodies.
// Skip the whole fence, including quote/list prefixes, before protecting math.
const CONTAINER_PREFIX = String.raw`(?:(?: {0,3}>[ \t]?)|(?: {0,3}(?:[-+*]|\d+[.)])[ \t]+))* {0,3}`;
const FENCE_OPENING = new RegExp(`^${CONTAINER_PREFIX}(\x60{3,}|~{3,})[^\\n]*(?:\\n|$)`, "u");

function fenceLength(source: string, position: number): number | "incomplete" | undefined {
  if (position > 0 && source[position - 1] !== "\n") return undefined;
  const opening = FENCE_OPENING.exec(source.slice(position));
  if (!opening) return undefined;
  const marker = opening[1];
  const closing = new RegExp(
    `^${CONTAINER_PREFIX}${marker[0]}{${marker.length},}[ \\t]*(?:\\n|$)`,
    "mu",
  );
  const end = closing.exec(source.slice(position + opening[0].length));
  return end ? opening[0].length + end.index + end[0].length : "incomplete";
}

function indentedLineLength(source: string, position: number): number {
  if (position > 0 && source[position - 1] !== "\n") return 0;
  return /^(?: {4}|\t)[^\n]*(?:\n|$)/u.exec(source.slice(position))?.[0].length ?? 0;
}

function inlineLiteralLength(source: string, position: number): number {
  const character = source[position];
  if (character === "`") return inline.matchInline?.(source, position)?.consumed ?? 0;
  if (character === "[" || character === "!" || character === "<") {
    return links.matchInline?.(source, position)?.consumed ?? 0;
  }
  return 0;
}

/** Last line boundary outside unfinished syntax; consumed upstream bytes cannot be reparsed. */
export function streamingCommitBoundary(source: string): number {
  let boundary = 0;
  for (let position = 0; position < source.length;) {
    const indented = indentedLineLength(source, position);
    if (indented) {
      position += indented;
      if (source[position - 1] === "\n") boundary = position;
      continue;
    }
    const fence = fenceLength(source, position);
    if (fence === "incomplete") return boundary;
    if (fence !== undefined) {
      position += fence;
      if (source[position - 1] === "\n") boundary = position;
      continue;
    }
    const literalLength = inlineLiteralLength(source, position);
    if (literalLength > 0) {
      position += literalLength;
      continue;
    }
    if (source[position] === "`") return boundary;
    const opening = source.slice(position, position + 2);
    if (opening === "\\(" || opening === "\\[" || opening === "$$") {
      const math = readMathSpan(source.slice(position));
      if (math) {
        position += math.raw.length;
        continue;
      }
      // Inline math cannot span a newline. Its entire line remains literal.
      const newline = source.indexOf("\n", position);
      if (opening !== "\\(" || newline < 0) return boundary;
      position = newline + 1;
      boundary = position;
      continue;
    }
    if (source[position] === "\n") boundary = position + 1;
    position += source[position] === "\\" && position + 1 < source.length ? 2 : 1;
  }
  return boundary;
}

/** Character references resolve to text before Generative DOM's inline matchers. */
export function protectStreamingMathSource(source: string): string {
  let protectedSource = "";
  for (let position = 0; position < source.length;) {
    const indented = indentedLineLength(source, position);
    if (indented) {
      protectedSource += source.slice(position, position + indented);
      position += indented;
      continue;
    }
    const fence = fenceLength(source, position);
    if (fence !== undefined) {
      const length = fence === "incomplete" ? source.length - position : fence;
      protectedSource += source.slice(position, position + length);
      position += length;
      continue;
    }
    const literalLength = inlineLiteralLength(source, position);
    if (literalLength > 0) {
      protectedSource += source.slice(position, position + literalLength);
      position += literalLength;
      continue;
    }
    const opening = source.slice(position, position + 2);
    if (opening === "\\(" || opening === "\\[" || opening === "$$") {
      const math = readMathSpan(source.slice(position));
      const newline = source.indexOf("\n", position);
      const raw =
        math?.raw ??
        source.slice(position, opening === "\\(" && newline >= 0 ? newline : undefined);
      protectedSource += Array.from(raw, (character) => `&#${character.codePointAt(0)};`).join("");
      position += raw.length;
      continue;
    }
    const length = source[position] === "\\" && position + 1 < source.length ? 2 : 1;
    protectedSource += source.slice(position, position + length);
    position += length;
  }
  return protectedSource;
}
