import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";
import { TEST_DISCOVERY_EXCLUDES } from "./scripts/typescript-test-paths.ts";

const ownerRoot = (path: string) => fileURLToPath(new URL(path, import.meta.url));

export default defineConfig({
  root: ownerRoot("."),
  test: {
    projects: [
      {
        extends: false,
        root: ownerRoot("."),
        test: {
          name: "repository",
          include: ["scripts/**/*.test.ts", "mise-tasks/**/*.test.ts", "tests/**/*.ts"],
          exclude: [...TEST_DISCOVERY_EXCLUDES],
        },
      },
      {
        extends: ownerRoot("./apps/desktop/vite.config.ts"),
        root: ownerRoot("./apps/desktop/"),
        test: {
          name: "desktop",
          include: ["src/**/*.test.ts", "tooling/**/*.test.ts", "tests/**/*.ts"],
          exclude: [...TEST_DISCOVERY_EXCLUDES],
          setupFiles: ["./tooling/browser-observers.ts"],
          globalSetup: ["./tooling/prerender/test-setup.ts"],
        },
      },
      {
        extends: false,
        root: ownerRoot("./packages/adapter-mcp-apps-host/"),
        test: {
          name: "mcp-apps-host",
          include: ["src/**/*.test.ts", "tests/**/*.ts"],
          exclude: [...TEST_DISCOVERY_EXCLUDES],
        },
      },
      {
        extends: false,
        root: ownerRoot("./packages/adapter-mcp-apps-view/"),
        test: {
          name: "mcp-apps-view",
          include: ["src/**/*.test.ts", "tests/**/*.ts"],
          exclude: [...TEST_DISCOVERY_EXCLUDES],
        },
      },
      {
        extends: false,
        root: ownerRoot("./packages/adapter-math-renderer/"),
        test: {
          name: "math-renderer",
          include: ["src/**/*.test.ts", "tests/**/*.ts"],
          exclude: [...TEST_DISCOVERY_EXCLUDES],
        },
      },
    ],
  },
});
