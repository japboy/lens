import {
  applyPresentationContext,
  presentationContextForPage,
  type AppView,
} from "../presentation-context";

/** The only critical module dependency is the finite document/native context contract. */
export async function startPage(view: AppView, load: () => Promise<unknown>): Promise<void> {
  try {
    const context = presentationContextForPage(
      view,
      document.documentElement.dataset.view,
      window.location.search,
    );
    const viewElement = document.querySelector<HTMLElement>(`lens-${view}-view`);
    applyPresentationContext(context, [
      document.documentElement,
      document.body,
      ...(viewElement ? [viewElement] : []),
    ]);
    await import("@lit-labs/ssr-client/lit-element-hydrate-support.js");
    await load();
    const page = document.querySelector<HTMLElement & { initialize?: () => Promise<void> }>(
      `lens-${view}-page`,
    );
    if (typeof page?.initialize !== "function") throw new Error(`Missing page owner: ${view}`);
    await page.initialize();
  } catch (error) {
    console.error("Unable to initialize page", error);
    const region = document.querySelector<HTMLElement>("[data-page-error]");
    if (!region) throw error;
    region.hidden = false;
    region
      .querySelector("button")
      ?.addEventListener("click", () => window.location.reload(), { once: true });
  }
}
