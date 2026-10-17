import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it, vi } from "vitest";

function links() {
  let pointer!: typeof handler;
  let selected = "";
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
      URL: "http://127.0.0.1/document/selected",
      baseURI: "http://127.0.0.1/document/selected",
      getSelection: () => ({ toString: () => selected }),
      addEventListener: (type: string, callback: typeof handler) => {
        if (type === "click") handler = callback;
        if (type === "pointerup") pointer = callback;
      },
    },
    Element,
    URL,
    parent,
    window: { addEventListener() {} },
    setTimeout: () => 0,
    clearTimeout() {},
  });
  parent.postMessage.mockClear();
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
    select: (text: string, trusted = true) => {
      selected = text;
      pointer({
        isTrusted: trusted,
        target: null,
        defaultPrevented: false,
        button: 0,
        preventDefault() {},
      });
    },
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
  it("reports only trusted bounded text selection through the standard context envelope", () => {
    const test = links();
    test.select("Selected phrase", false);
    expect(test.parent.postMessage).not.toHaveBeenCalled();
    test.select("x".repeat(5000));
    expect(test.parent.postMessage).toHaveBeenCalledWith(
      {
        jsonrpc: "2.0",
        id: "lens-selection-1",
        method: "ui/update-model-context",
        params: { structuredContent: { lensSelection: "x".repeat(4000) } },
      },
      "*",
    );
  });
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
      "http://127.0.0.1/document/selected#section",
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
