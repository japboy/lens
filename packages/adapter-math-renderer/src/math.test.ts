// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { Marked, type Token } from "marked";
import { excludeHtmlMath, mathElement, readMathSpan, renderMathMarkup } from "./math";

describe("shared bounded math rendering", () => {
  it("recognizes complete inline and display spans while leaving incomplete text literal", () => {
    expect(readMathSpan(String.raw`\(x\) suffix`)).toEqual({
      raw: String.raw`\(x\)`,
      tex: "x",
      displayMode: false,
    });
    expect(readMathSpan("$$x\ny$$")?.displayMode).toBe(true);
    expect(readMathSpan("\\(x\ny\\)")).toBeUndefined();
    expect(readMathSpan(String.raw`\(unfinished`)).toBeUndefined();
  });
  it("shares trust and expansion restrictions between markup and DOM renderers", () => {
    const trusted = readMathSpan(String.raw`\(\href{javascript:alert(1)}{x}\)`)!;
    expect(renderMathMarkup(trusted)).not.toContain("href=");
    expect(mathElement(trusted, true).querySelector("a")).toBeNull();
    const recursive = readMathSpan(String.raw`\(\def\a{\a}\a\)`)!;
    expect(() => renderMathMarkup(recursive)).toThrow(/Too many expansions/u);
    expect(mathElement(recursive, true).textContent).toBe(recursive.raw);
  });
  it("does not carry a global macro definition into a later expression", () => {
    renderMathMarkup(readMathSpan(String.raw`\(\gdef\shared{x}\shared\)`)!);
    const independent = readMathSpan(String.raw`\(\shared\)`)!;
    expect(() => renderMathMarkup(independent)).toThrow(/Undefined control sequence/u);
    expect(mathElement(independent, true).textContent).toBe(independent.raw);
  });
  it("handles escaped closing delimiters and disallows inline newlines", () => {
    expect(readMathSpan(String.raw`\(a\\)b\)`)?.tex).toBe(String.raw`a\\)b`);
    expect(readMathSpan("\\(a\nb\\)")).toBeUndefined();
  });
  it("uses the local parser's extension child fields in synchronous source order", () => {
    const parser = new Marked({ async: false });
    parser.use({
      extensions: [
        {
          name: "fixture",
          childTokens: ["content"],
          level: "block",
          tokenizer: () => undefined,
          renderer: () => "",
        },
      ],
    });
    const html = (raw: string): Token => ({ type: "html", raw, text: raw, block: false });
    const math = (tex: string): Token & { literal?: boolean } => ({
      type: "lensMath",
      raw: String.raw`\(${tex}\)`,
      tex,
    });
    const hidden = math("hidden");
    const visible = math("visible");
    const ignored = math("ignored");
    const tokens: Token[] = [
      {
        type: "fixture",
        raw: "",
        content: [html("<span>"), [hidden, html("</span>"), visible]],
        tokens: [html("<span>"), ignored],
      },
    ];
    const outside = math("outside");
    tokens.push(outside);
    excludeHtmlMath(parser, tokens);
    expect(hidden.literal).toBe(true);
    expect(visible.literal).toBeUndefined();
    expect(ignored.literal).toBeUndefined();
    expect(outside.literal).toBeUndefined();
  });
});
