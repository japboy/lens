import { BUILD_PATHS } from "../../tooling/build-paths";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { AppView } from "../page-entries";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "../..");

/** jsdom does not parse native DSD. Only the test adapter performs this parser step. */
export function installGeneratedPage(view: AppView): HTMLElement {
  const html = readFileSync(resolve(desktop, BUILD_PATHS.tests, `${view}.html`), "utf8");
  const parsed = new DOMParser().parseFromString(html, "text/html");
  const page = parsed.querySelector<HTMLElement>(`lens-${view}-page`);
  if (!page) throw new Error("Missing generated page");
  function attachRoots(container: ParentNode): void {
    for (const template of container.querySelectorAll<HTMLTemplateElement>(
      'template[shadowrootmode="open"]',
    )) {
      const host = template.parentElement!;
      const root = host.attachShadow({ mode: "open" });
      root.append(template.content);
      template.remove();
      attachRoots(root);
    }
  }
  attachRoots(page);
  document.documentElement.dataset.view = view;
  window.history.replaceState({}, "", `/${view}.html?platform=macos`);
  document.body.replaceChildren(page);
  const recovery = parsed.querySelector<HTMLElement>("[data-page-error]");
  if (recovery) document.body.append(recovery);
  return page;
}
