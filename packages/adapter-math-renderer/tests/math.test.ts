// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { mathElement, readMathSpan, renderMathMarkup } from "../src/math";

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
});
