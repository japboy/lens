import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import { BUILD_PATHS } from "./tooling/build-paths.ts";
import { PAGE_ENTRIES } from "./src/page-entries.js";

export default defineConfig({
  clearScreen: false,
  test: {
    include: ["src/**/*.test.ts", "tooling/**/*.test.ts"],
    globalSetup: ["./tooling/prerender/test-setup.ts"],
  },
  build: {
    outDir: BUILD_PATHS.webview,
    manifest: true,
    rolldownOptions: {
      input: Object.fromEntries(
        Object.entries(PAGE_ENTRIES).map(([view, path]) => [
          view,
          fileURLToPath(new URL(path, import.meta.url)),
        ]),
      ),
    },
  },
  server: { port: 1420, strictPort: true },
});
