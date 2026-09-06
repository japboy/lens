import { createServer, type ServerResponse } from "node:http";
import { watch } from "node:fs";
import { readFile, mkdir, mkdtemp, rm, rename } from "node:fs/promises";
import { resolve, dirname, extname, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { BUILD_PATHS } from "../build-paths.ts";
import { PAGE_ENTRIES } from "../../src/page-entries.ts";

const app = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const repo = resolve(app, "../..");
const generations = resolve(app, BUILD_PATHS.development);
await mkdir(generations, { recursive: true });
const directory = await mkdtemp(resolve(generations, "run-"));
const clients = new Set<ServerResponse>();
let current: { id: string; path: string } | undefined;
let previous: typeof current;
let building = false;
let pending = false;
let stopped = false;
let worker: ReturnType<typeof spawn> | undefined;

function run(output: string): Promise<void> {
  return new Promise((resolveRun, reject) => {
    worker = spawn(
      process.execPath,
      [resolve(app, "tooling/prerender/build.ts"), "--development", `--output=${output}`],
      { cwd: app, stdio: "inherit" },
    );
    worker.once("error", reject);
    worker.once("exit", (code) => {
      worker = undefined;
      if (code === 0) resolveRun();
      else reject(new Error(`Generation exited with ${code}`));
    });
  });
}

async function rebuild(): Promise<void> {
  pending = true;
  if (building || stopped) return;
  building = true;
  try {
    while (pending && !stopped) {
      pending = false;
      const output = await mkdtemp(resolve(directory, "build-"));
      try {
        await run(output);
        if (pending || stopped) {
          await rm(output, { recursive: true, force: true });
          continue;
        }
        const metadata = JSON.parse(await readFile(resolve(output, "generation.json"), "utf8")) as {
          generation: string;
        };
        if (metadata.generation === current?.id) {
          await rm(output, { recursive: true, force: true });
          continue;
        }
        const destination = resolve(directory, metadata.generation);
        if (metadata.generation === previous?.id) {
          // A source revert reuses immutable bytes still serving the previous document.
          await rm(output, { recursive: true, force: true });
        } else {
          await rename(output, destination);
        }
        const obsolete = previous;
        previous = current;
        current = { id: metadata.generation, path: destination };
        for (const response of clients) response.write(`data: ${current.id}\n\n`);
        if (obsolete && obsolete.path !== current.path)
          await rm(obsolete.path, { recursive: true, force: true });
        console.log(`Published WebView generation ${current.id}`);
      } catch (error) {
        await rm(output, { recursive: true, force: true });
        console.error(error);
        if (!current && !pending) throw error;
      }
    }
  } finally {
    building = false;
  }
}

const watchedAppFile = (file: string) =>
  /^(src\/|tooling\/|src-tauri\/icons\/|package\.json$|tsconfig[^/]*\.json$)/.test(file);
const watchers = [
  watch(app, { recursive: true }, (_event, file) => {
    if (file && watchedAppFile(file.toString().replaceAll("\\", "/"))) void rebuild();
  }),
  watch(repo, (_event, file) => {
    if (file && ["pnpm-lock.yaml", "pnpm-workspace.yaml", "package.json"].includes(file.toString()))
      void rebuild();
  }),
  watch(resolve(repo, "packages/typescript-config"), { recursive: true }, () => {
    void rebuild();
  }),
];
await rebuild();

const types: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json",
  ".map": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".woff2": "font/woff2",
  ".woff": "font/woff",
};
const reloader = `const source = new EventSource("/_development/events"); source.onmessage = event => { if (event.data !== document.documentElement.dataset.generation) location.reload(); };`;

const server = createServer(async (request, response) => {
  try {
    const pathname = decodeURIComponent(new URL(request.url ?? "/", "http://localhost").pathname);
    if (pathname === "/_development/events") {
      response.writeHead(200, {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-store",
        Connection: "keep-alive",
      });
      clients.add(response);
      if (current) response.write(`data: ${current.id}\n\n`);
      request.on("close", () => clients.delete(response));
      return;
    }
    if (pathname === "/_development/reload.js") {
      response.writeHead(200, { "Content-Type": "text/javascript", "Cache-Control": "no-store" });
      response.end(reloader);
      return;
    }
    const page = Object.values(PAGE_ENTRIES).find((file) => pathname === `/${file}`);
    if (page && current) {
      const selected = current;
      const html = (await readFile(resolve(selected.path, page), "utf8"))
        .replace("<html ", `<html data-generation="${selected.id}" `)
        .replace(
          "</body>",
          '<script type="module" async src="/_development/reload.js"></script></body>',
        );
      response.writeHead(200, { "Content-Type": types[".html"]!, "Cache-Control": "no-store" });
      response.end(html);
      return;
    }
    const match = /^\/_generations\/([a-f0-9]{64})\/(.+)$/.exec(pathname);
    const selected = match && [current, previous].find((item) => item?.id === match[1]);
    if (!match || !selected) {
      response.writeHead(404);
      response.end("Unknown generation");
      return;
    }
    const file = resolve(selected.path, match[2]!);
    if (!file.startsWith(selected.path + sep)) {
      response.writeHead(404);
      response.end();
      return;
    }
    const bytes = await readFile(file);
    response.writeHead(200, {
      "Content-Type": types[extname(file)] ?? "application/octet-stream",
      "Cache-Control": "public, max-age=31536000, immutable",
    });
    response.end(bytes);
  } catch (error) {
    response.writeHead((error as NodeJS.ErrnoException).code === "ENOENT" ? 404 : 500);
    response.end("Unable to serve generated resource");
  }
});
server.listen(1420, "127.0.0.1", () => console.log("Generated WebViews: http://localhost:1420"));

for (const signal of ["SIGINT", "SIGTERM"] as const)
  process.on(signal, () => {
    stopped = true;
    worker?.kill(signal);
    for (const watcher of watchers) watcher.close();
    for (const response of clients) response.end();
    server.close(() => {
      void rm(directory, { recursive: true, force: true }).catch(console.error);
    });
  });
