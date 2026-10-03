import { defineConfig, type Plugin } from "vite";
/** Preview consumes public UI exports; build fails if native authority leaks in. */
function previewBoundary(): Plugin {
  return {
    name: "preview-native-boundary",
    apply: "build",
    moduleParsed(info) {
      if (/apps\/desktop|@tauri-apps|src-tauri|virtual:lens/.test(info.id))
        throw new Error(`Native dependency in UI preview: ${info.id}`);
    },
  };
}
export default defineConfig({
  plugins: [previewBoundary()],
  build: { outDir: ".build/preview", emptyOutDir: true },
  server: { host: "127.0.0.1", port: 4183, strictPort: true },
});
