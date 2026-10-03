import { createServer, type ServerResponse } from "node:http";
import { watch } from "node:fs";
import { readFile, mkdir, mkdtemp, rm, rename } from "node:fs/promises";
import { resolve, extname, sep } from "node:path";
import { spawn } from "node:child_process";
export type DevelopmentOptions<Resources> = {
  outputRoot: string;
  pages: readonly string[];
  port: number;
  host: string;
  buildCommand: { command: string; arguments: readonly string[]; cwd: string };
  watchRoots: readonly { directory: string; accepts: (relativePath: string) => boolean }[];
  readResources: (directory: string) => Resources;
  resourceHeaders: (resources: Resources, path: string, bytes: Buffer) => Record<string, string>;
};
export async function startDevelopmentServer<Resources>(
  options: DevelopmentOptions<Resources>,
): Promise<{ url: string; close: () => Promise<void> }> {
  await mkdir(options.outputRoot, { recursive: true });
  const directory = await mkdtemp(resolve(options.outputRoot, "run-"));
  const clients = new Set<ServerResponse>();
  let current: { id: string; path: string; resources: Resources } | undefined;
  let previous: typeof current;
  let building = false;
  let pending = false;
  let stopped = false;
  let worker: ReturnType<typeof spawn> | undefined;

  function run(output: string): Promise<void> {
    return new Promise((resolveRun, reject) => {
      worker = spawn(
        options.buildCommand.command,
        [...options.buildCommand.arguments, `--output=${output}`],
        { cwd: options.buildCommand.cwd, stdio: "inherit" },
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
          const metadata = JSON.parse(
            await readFile(resolve(output, "generation.json"), "utf8"),
          ) as {
            generation: string;
          };
          if (!/^[a-f0-9]{64}$/.test(metadata.generation))
            throw new Error("Invalid generation identity");
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
          const resources = options.readResources(destination);
          const obsolete = previous;
          previous = current;
          current = {
            id: metadata.generation,
            path: destination,
            resources,
          };
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

  const watchers = options.watchRoots.map(({ directory, accepts }) =>
    watch(directory, { recursive: true }, (_event, file) => {
      if (file && accepts(file.toString().replaceAll("\\", "/"))) void rebuild();
    }),
  );
  try {
    await rebuild();
  } catch (error) {
    stopped = true;
    for (const watcher of watchers) watcher.close();
    await rm(directory, { recursive: true, force: true });
    throw error;
  }

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
      const page = options.pages.find((file) => pathname === `/${file}`);
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
      const mathHeaders =
        request.method === "GET" || request.method === "HEAD"
          ? options.resourceHeaders(selected.resources, match[2]!, bytes)
          : {};
      response.writeHead(200, {
        "Content-Type": types[extname(file)] ?? "application/octet-stream",
        "Cache-Control": "public, max-age=31536000, immutable",
        ...mathHeaders,
      });
      response.end(bytes);
    } catch (error) {
      response.writeHead((error as NodeJS.ErrnoException).code === "ENOENT" ? 404 : 500);
      response.end("Unable to serve generated resource");
    }
  });
  try {
    await new Promise<void>((resolveListen, reject) => {
      server.once("error", reject);
      server.listen(options.port, options.host, resolveListen);
    });
  } catch (error) {
    stopped = true;
    for (const watcher of watchers) watcher.close();
    await rm(directory, { recursive: true, force: true });
    throw error;
  }
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("Unexpected development server address");
  const url = `http://${options.host}:${address.port}`;
  console.log(`Generated WebViews: ${url}`);
  return {
    url,
    close: async () => {
      stopped = true;
      if (worker) {
        const running = worker;
        const exited = new Promise<void>((resolveExit) =>
          running.once("exit", () => resolveExit()),
        );
        running.kill("SIGTERM");
        await exited;
      }
      for (const watcher of watchers) watcher.close();
      for (const response of clients) response.end();
      await new Promise<void>((resolveClose) => server.close(() => resolveClose()));
      await rm(directory, { recursive: true, force: true });
    },
  };
}
