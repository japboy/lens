import { defineConfig } from "oxlint";

export default defineConfig({
  plugins: ["eslint", "typescript", "unicorn", "oxc", "import", "vitest"],
  categories: {
    correctness: "error",
  },
  env: {
    browser: true,
    es6: true,
  },
  options: {
    denyWarnings: true,
    reportUnusedDisableDirectives: "error",
  },
  overrides: [
    {
      files: ["*.config.ts", "apps/desktop/*.config.ts", "scripts/**/*.ts", "mise-tasks/**/*.ts"],
      env: {
        browser: false,
        node: true,
      },
    },
    {
      files: ["apps/desktop/src/**/*.test.ts"],
      env: {
        vitest: true,
      },
    },
  ],
});
