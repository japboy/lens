import { readFile, readdir } from "node:fs/promises";
import { resolve } from "node:path";
import { expect, it } from "vitest";
import { BUILD_PATHS } from "../tooling/build-paths";
it("ships an ES module preparation worker matching its constructor", async () => {
  const assets = resolve(BUILD_PATHS.tests, "assets");
  const files = await readdir(assets);
  const worker = files.find((file) => /^conversation-html\.worker-.*\.js$/.test(file));
  expect(worker).toBeDefined();
  const source = await readFile(resolve(assets, worker!), "utf8");
  expect(source.trimStart()).not.toMatch(/^\(function\s*\(/);
  expect(source).toContain("onmessage");
  const component = files.find((file) => /^lens-session-document-.*\.js$/.test(file));
  const caller = await readFile(resolve(assets, component!), "utf8");
  expect(caller).toMatch(/type:\s*[`"']module[`"']/);
});
