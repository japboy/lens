import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vitest";

function links() {
  let handler!: (event: {
    isTrusted: boolean;
    defaultPrevented: boolean;
    button: number;
    target: unknown;
    preventDefault(): void;
  }) => void;
  const parent = { postMessage: vi.fn<Window["postMessage"]>() };
  let href = "https://example.com/path?q=73#part";
  let download = false;
  class Element {
    closest() {
      return this;
    }
    getAttribute() {
      return href;
    }
    hasAttribute() {
      return download;
    }
  }
  runInNewContext(readFileSync(new URL("./rich-html-links.js", import.meta.url), "utf8"), {
    document: {
      addEventListener: (_: string, callback: typeof handler) => {
        handler = callback;
      },
    },
    Element,
    URL,
    parent,
  });
  const click = (options: Partial<Parameters<typeof handler>[0]> = {}) => {
    const preventDefault = vi.fn<() => void>();
    handler({
      isTrusted: true,
      defaultPrevented: false,
      button: 0,
      target: new Element(),
      preventDefault,
      ...options,
    });
    return preventDefault;
  };
  return {
    parent,
    click,
    href: (value: string) => {
      href = value;
    },
    download: () => {
      download = true;
    },
  };
}
describe("built-in document link helper", () => {
  it("turns a real absolute HTTP(S) anchor click into standard protocol only", () => {
    const test = links();
    expect(test.click()).toHaveBeenCalledOnce();
    expect(test.parent.postMessage).toHaveBeenCalledWith(
      {
        jsonrpc: "2.0",
        id: "lens-link-1",
        method: "ui/open-link",
        params: { url: "https://example.com/path?q=73#part" },
      },
      "*",
    );
  });
  it("preserves author cancellation, local anchors, downloads and non-web destinations", () => {
    const test = links();
    expect(test.click({ isTrusted: false })).not.toHaveBeenCalled();
    expect(test.click({ defaultPrevented: true })).not.toHaveBeenCalled();
    expect(test.click({ button: 1 })).not.toHaveBeenCalled();
    for (const href of [
      "#section",
      "/relative",
      "file:///private",
      "javascript:alert(1)",
      "mailto:a@example.com",
      "https://user:secret@example.com",
    ]) {
      test.href(href);
      expect(test.click()).not.toHaveBeenCalled();
    }
    test.href("https://example.com");
    test.download();
    expect(test.click()).not.toHaveBeenCalled();
    expect(test.parent.postMessage).not.toHaveBeenCalled();
  });
});
