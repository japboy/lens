// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { renderMarkdownFragment } from "./markdown";
import { MATH_LIMITS, readMathSpan } from "./markdown-math";

function render(source: string): HTMLDivElement {
  const host = document.createElement("div");
  host.append(renderMarkdownFragment(source));
  return host;
}

describe("Markdown math", () => {
  it("renders local accessible inline and display math without consuming emphasis", () => {
    const host = render(String.raw`**before \(a_b * c\) after**

\[\frac{1}{2}\]

$$x^2$$`);
    expect(host.querySelectorAll(".katex")).toHaveLength(3);
    expect(host.querySelectorAll("math")).toHaveLength(3);
    expect(host.querySelectorAll(".katex-display")).toHaveLength(2);
    expect(host.querySelector("strong .katex")).not.toBeNull();
  });

  it("excludes code, raw HTML, links and monetary dollars", () => {
    const host = render(
      String.raw`Price $5 and $10. \$\$escaped

` +
        "`\\(code\\)`" +
        String.raw`

<span><b>\(raw\)</b></span> [\(label\)](https://example.com/)

<div>\(block\)</div>

` +
        "```tex\n\\(fenced\\)\n```",
    );
    expect(host.querySelector(".katex")).toBeNull();
    expect(host.textContent).toContain("$5 and $10");
    expect(host.textContent).toContain(String.raw`\(raw\)`);
  });

  it("does not authorize math from forged markers or HTML entities", () => {
    const host = render(
      String.raw`LENSMATH0_0END &#76;ENSMATH1_0END <span data-lens-math="0">LENSMATHXX0END</span> \(x\)`,
    );
    expect(host.querySelectorAll(".katex")).toHaveLength(1);
    expect(host.textContent).toContain("LENSMATH0_0END LENSMATH1_0END LENSMATHXX0END");
  });

  it("retains incomplete and invalid expressions as inert original text", () => {
    const host = render(String.raw`\(\notACommand{<img src=x onerror=alert(1)>}\)

\(unfinished_a*

\[unterminated`);
    expect(host.querySelector("img")).toBeNull();
    expect(host.textContent).toContain(String.raw`\(unfinished_a*`);
    expect(host.textContent).toContain(String.raw`\[unterminated`);
    expect(host.querySelector(".lens-math-fallback")?.textContent).toContain("<img");
  });

  it("isolates macros and refuses trusted commands", () => {
    const host = render(
      String.raw`\(\gdef\foo{X}\foo\) \(\foo\) \(\href{javascript:alert(1)}{X}\)`,
    );
    expect(host.querySelector("a")).toBeNull();
    expect(host.querySelectorAll(".lens-math-fallback").length).toBeGreaterThanOrEqual(1);
  });

  it("bounds rendering work and retains excess input", () => {
    const raw = `\\(${"x".repeat(MATH_LIMITS.characters + 1)}\\)`;
    expect(render(raw).querySelector(".lens-math-fallback")?.textContent).toBe(raw);
    const host = render(
      Array.from({ length: MATH_LIMITS.count + 1 }, () => String.raw`\(x\)`).join(" "),
    );
    expect(host.querySelectorAll(".katex")).toHaveLength(MATH_LIMITS.count);
    expect(host.querySelectorAll(".lens-math-fallback")).toHaveLength(1);
  });

  it("handles escaped closing delimiters and disallows inline newlines", () => {
    expect(readMathSpan(String.raw`\(a\\)b\)`)?.tex).toBe(String.raw`a\\)b`);
    expect(readMathSpan("\\(a\nb\\)")).toBeUndefined();
  });
  it("keeps TeX emphasis characters opaque and preserves ordinary escaped text emphasis", () => {
    const host = render(
      String.raw`*before \(x * y\) after* and \\(ordinary **bold**\) and ` +
        "`\\(unclosed`" +
        " **outside**",
    );
    expect(host.querySelector("em .katex")).not.toBeNull();
    expect([...host.querySelectorAll("strong")].map((node) => node.textContent)).toEqual([
      "bold",
      "outside",
    ]);
  });

  it("renders multiline matrices, including blank lines, and protects nested HTML", () => {
    const host = render(String.raw`$$
\begin{matrix}a & b \\ c & d\end{matrix}

$$

<span>**\(hidden\)**</span>`);
    expect(host.querySelectorAll(".katex")).toHaveLength(1);
    expect(host.querySelector(".lens-math-display math")).not.toBeNull();
    expect(host.textContent).toContain(String.raw`\(hidden\)`);
  });

  it("blocks author style and external-image commands and terminates recursive macros", () => {
    const host = render(
      String.raw`\(\htmlStyle{position:fixed}{x}\) \(\includegraphics{https://example.com/x.png}\) \(\def\loop{\loop}\loop\)`,
    );
    expect(host.querySelector("img, [style*=fixed]")).toBeNull();
    expect(host.querySelectorAll(".lens-math-fallback").length).toBeGreaterThanOrEqual(1);
  });

  it("enforces the aggregate character budget independently of the expression count", () => {
    const tex = `x${" ".repeat(MATH_LIMITS.characters - 1)}`;
    const host = render(Array.from({ length: 9 }, () => `\\(${tex}\\)`).join(" "));
    expect(host.querySelectorAll(".katex")).toHaveLength(8);
    expect(host.querySelectorAll(".lens-math-fallback")).toHaveLength(1);
  });
  it("selects a collision-free marker from a long hostile prefix in one scan", () => {
    const hostile = `LENSMATH${"X".repeat(100_000)}0END`;
    const host = render(`${hostile} \\(x\\)`);
    expect(host.querySelectorAll(".katex")).toHaveLength(1);
    expect(host.textContent?.startsWith(hostile)).toBe(true);
  });
  it("does not interpret tag-looking text inside HTML comments or attributes as boundaries", () => {
    const host =
      render(String.raw`<span><!-- </span> --> \(hidden\)</span> <!-- <span> --> \(visible\)

<span title="<b> <i>">\(alsoHidden\)</span> \(alsoVisible\)`);
    expect(host.querySelectorAll(".katex")).toHaveLength(2);
    expect(host.textContent).toContain(String.raw`\(hidden\)`);
    expect(host.textContent).toContain(String.raw`\(alsoHidden\)`);
  });
  it("keeps malformed nonvoid HTML contexts opaque until their matching close", () => {
    const host = render(String.raw`<span></em>\(hidden\)</span> \(visible\)

<span/>\(alsoHidden\)</span> \(alsoVisible\)`);
    expect(host.querySelectorAll(".katex")).toHaveLength(2);
    expect(host.textContent).toContain(String.raw`\(hidden\)`);
    expect(host.textContent).toContain(String.raw`\(alsoHidden\)`);
  });
  it.each(["\\(x\\)", "- \\(x\\)", "$$x$$", "**\\(x\\)**"])(
    "keeps block HTML ownership across blank lines and nested Markdown: %s",
    (inside) => {
      const host = render(`<div>\n\n${inside}\n\n</div>\n\n\\(outside\\)`);
      expect(host.querySelector(":scope > div .katex")).toBeNull();
      expect(host.querySelectorAll(".katex")).toHaveLength(1);
      expect(host.querySelector(".katex annotation")?.textContent).toBe("outside");
      expect(host.querySelector("div")?.textContent).toContain(
        inside.replaceAll("**", "").replace("- ", ""),
      );
    },
  );

  it("tracks all tags in a block while ignoring comment and attribute lookalikes", () => {
    const host = render(String.raw`<div title="</div> > <aside>"><section><!-- </section></div> -->

\(inside\)

</section></div>

Text <!-- <div> --> \(outside\)`);
    expect(host.querySelector(":scope > div .katex")).toBeNull();
    expect(host.querySelectorAll(".katex")).toHaveLength(1);
    expect(host.querySelector(".katex annotation")?.textContent).toBe("outside");
  });

  it("does not treat tag-like script text as HTML boundaries", () => {
    const host = render(String.raw`<div><script>const fake = "</div>";</script>

\(inside\)

</div>

\(outside\)`);
    expect(host.querySelector(":scope > div .katex")).toBeNull();
    expect(host.querySelectorAll(".katex")).toHaveLength(1);
    expect(host.querySelector("script")).toBeNull();
  });
});
