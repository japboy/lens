import { defineConfig } from "oxfmt";

export default defineConfig({
  // Release Please owns the changelog format; do not rewrite its generated output.
  ignorePatterns: [
    "/CHANGELOG.md",
    "apps/desktop/src-tauri/gen/**",
    "apps/desktop/tests/fixtures/**",
  ],
});
