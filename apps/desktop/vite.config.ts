import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import { PAGE_ENTRIES } from "./src/page-entries.js";

export default defineConfig({
  clearScreen: false,
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
