import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: [
      "scripts/**/*.test.ts",
      "mise-tasks/**/*.test.ts",
      "apps/desktop/src-tauri/agent-runtime/**/*.test.ts",
    ],
  },
});
