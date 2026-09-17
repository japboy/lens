import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import { BUILD_PATHS } from "./tooling/build-paths.ts";
import { PAGE_ENTRIES } from "./src/page-entries.js";
import { htmlMathAssetsPlugin } from "./tooling/html-math-assets.ts";

const applicationRoot = fileURLToPath(new URL(".", import.meta.url));
const clientRoot = fileURLToPath(new URL("./src/", import.meta.url));

export default defineConfig({
  plugins: [await htmlMathAssetsPlugin(applicationRoot)],
  root: clientRoot,
  clearScreen: false,
  test: {
    root: applicationRoot,
    include: ["src/**/*.test.ts", "tooling/**/*.test.ts", "tests/*.ts"],
    setupFiles: ["./tooling/browser-observers.ts"],
    globalSetup: ["./tooling/prerender/test-setup.ts"],
  },
  build: {
    outDir: fileURLToPath(new URL(BUILD_PATHS.webview, import.meta.url)),
    emptyOutDir: true,
    manifest: true,
    rolldownOptions: {
      input: Object.fromEntries(
        Object.entries(PAGE_ENTRIES).map(([view, path]) => [
          view,
          fileURLToPath(new URL(`./src/${path}`, import.meta.url)),
        ]),
      ),
    },
  },
  server: { port: 1420, strictPort: true },
});
