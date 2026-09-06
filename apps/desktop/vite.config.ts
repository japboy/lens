import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import { PAGE_ENTRIES } from "./src/page-entries.js";

export default defineConfig({
  clearScreen: false,
  test: {
    include: ["src/**/*.test.ts", "tooling/**/*.test.ts"],
    globalSetup: ["./tooling/prerender/test-setup.ts"],
  },
  build: {
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
