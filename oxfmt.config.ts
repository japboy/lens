import { defineConfig } from "oxfmt";

export default defineConfig({
  // Release Please owns changelog and version-manifest formatting; do not rewrite its generated output.
  ignorePatterns: [
    "/CHANGELOG.md",
    "/.release-please-manifest.json",
    "apps/desktop/src-tauri/gen/**",
    "apps/desktop/tests/fixtures/**",
  ],
});
