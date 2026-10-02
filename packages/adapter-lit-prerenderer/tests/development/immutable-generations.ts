import { afterEach, expect, it } from "vitest";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { startDevelopmentServer } from "../../src/development.ts";
const cleanup: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const action of cleanup.splice(0).reverse()) await action();
});
async function eventually(action: () => Promise<boolean>): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (await action()) return;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error("Development publication did not reach expected state");
}
it("retains immutable current/previous resources and reuses a reverted generation", async () => {
  const directory = await mkdtemp(join(tmpdir(), "lit-generation-server-"));
  cleanup.push(() => rm(directory, { recursive: true, force: true }));
  const watched = join(directory, "source");
  await mkdir(watched);
  const input = join(watched, "value");
  await writeFile(input, "a");
  const worker = join(directory, "build.mjs");
  await writeFile(
    worker,
    `import { readFile, mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
const value = await readFile(${JSON.stringify(input)}, "utf8");
const generation = value.repeat(64);
const output = process.argv.find(value => value.startsWith("--output=")).slice(9);
await mkdir(join(output,"assets"), {recursive:true});
await writeFile(join(output,"page.html"), '<html ><body>'+value+'</body></html>');
await writeFile(join(output,"assets/value.txt"), value);
await writeFile(join(output,"generation.json"), JSON.stringify({generation}));`,
  );
  const server = await startDevelopmentServer({
    outputRoot: join(directory, "generations"),
    pages: ["page.html"],
    host: "127.0.0.1",
    port: 0,
    buildCommand: { command: process.execPath, arguments: [worker], cwd: directory },
    watchRoots: [{ directory: watched, accepts: (path) => path === "value" }],
    readResources: () => undefined,
    resourceHeaders: () => ({}),
  });
  cleanup.push(server.close);
  const html = () => fetch(`${server.url}/page.html`).then((response) => response.text());
  const asset = (id: string) =>
    fetch(`${server.url}/_generations/${id.repeat(64)}/assets/value.txt`);
  expect(await (await asset("a")).text()).toBe("a");
  await writeFile(input, "b");
  await eventually(async () => (await html()).includes('data-generation="' + "b".repeat(64)));
  expect(await (await asset("a")).text()).toBe("a");
  expect(await (await asset("b")).text()).toBe("b");
  await writeFile(input, "a");
  await eventually(async () => (await html()).includes('data-generation="' + "a".repeat(64)));
  expect(await (await asset("b")).text()).toBe("b");
  await writeFile(input, "c");
  await eventually(async () => (await html()).includes('data-generation="' + "c".repeat(64)));
  expect((await asset("b")).status).toBe(404);
  expect(await (await asset("a")).text()).toBe("a");
  expect((await fetch(`${server.url}/private`)).status).toBe(404);
});
