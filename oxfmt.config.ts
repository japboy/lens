import { defineConfig } from "oxfmt";

export default defineConfig({
  // Release Please owns these release-file formats, including Tauri JSON serialization.
  ignorePatterns: [
    "/CHANGELOG.md",
    "/.release-please-manifest.json",
    "/apps/desktop/src-tauri/tauri.conf.json",
    "apps/desktop/src-tauri/gen/**",
    "apps/desktop/tests/fixtures/**",
  ],
});
