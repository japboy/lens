import type { LitElement } from "lit";

/** Adopt only generated native DSD. A mismatch is an initialization failure. */
export async function hydrateView(view: LitElement): Promise<void> {
  const root = view.shadowRoot;
  const generated =
    root &&
    Array.from(root.childNodes).some(
      (node) => node.nodeType === Node.COMMENT_NODE && node.nodeValue?.startsWith("lit-part "),
    );
  if (!root || !generated || !view.hasAttribute("defer-hydration"))
    throw new Error("Missing deferred Declarative Shadow DOM view");
  view.removeAttribute("defer-hydration");
  await view.updateComplete;
  if (view.shadowRoot !== root) throw new Error("Hydration replaced the initial shadow root");
}
