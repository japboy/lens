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
    applyPresentationContext(context, [document.documentElement, document.body]);
    await load();
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
