import { relative, resolve } from "node:path";

export const BUILD_PATHS = {
  webview: ".build/webview",
  development: ".build/development",
  tests: ".build/tests",
  staging: ".build/staging",
} as const;

/** Only complete output trees may be replaced; staging and caches have separate owners. */
export function assertGenerationOutput(desktop: string, output: string): void {
  const path = relative(desktop, resolve(output)).replaceAll("\\", "/");
  if (
    path === BUILD_PATHS.webview ||
    path === BUILD_PATHS.tests ||
    path.startsWith(BUILD_PATHS.tests + "/") ||
    path.startsWith(BUILD_PATHS.development + "/")
  )
    return;
  throw new Error("Output must be the WebView distribution or an owned development/test directory");
}
