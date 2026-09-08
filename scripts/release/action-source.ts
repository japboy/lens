import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parse } from "yaml";

const action = "googleapis/release-please-action";

function mapping(value: unknown): Record<string, unknown> {
  assert.ok(value && typeof value === "object" && !Array.isArray(value), "Expected YAML mapping");
  return value as Record<string, unknown>;
}

export function actionRevision(workflow: string): string {
  const document = mapping(parse(workflow));
  const references: string[] = [];
  for (const job of Object.values(mapping(document.jobs))) {
    const steps = mapping(job).steps;
    if (steps === undefined) continue;
    assert.ok(Array.isArray(steps), "Expected workflow steps");
    for (const step of steps) {
      const uses = mapping(step).uses;
      if (typeof uses === "string" && uses.split("@")[0] === action) references.push(uses);
    }
  }
  assert.equal(references.length, 1, "Expected exactly one Release Please Action step");
  const match = /^googleapis\/release-please-action@([0-9a-f]{40})$/u.exec(references[0]!);
  assert.ok(match, "Release Please Action must use a full commit SHA");
  return match[1]!;
}

export async function withActionSource<T>(
  revision: string,
  templates: string[],
  verify: (source: { directory: string; bundle: string }) => Promise<T>,
  retrieve: typeof fetch = fetch,
): Promise<T> {
  assert.match(revision, /^[0-9a-f]{40}$/u);
  assert.equal(new Set(templates).size, templates.length, "Duplicate Action template");
  for (const name of templates) assert.match(name, /^[a-z]+[0-9]?\.hbs$/u);
  const directory = mkdtempSync(join(tmpdir(), "lens-release-please-"));
  try {
    for (const name of ["index.js", ...templates]) {
      const response = await retrieve(
        `https://raw.githubusercontent.com/${action}/${revision}/dist/${name}`,
        { signal: AbortSignal.timeout(60_000), redirect: "error" },
      );
      assert.ok(response.ok, `Could not retrieve Action ${name}: ${response.status}`);
      writeFileSync(
        join(directory, name === "index.js" ? "index.cjs" : name),
        Buffer.from(await response.arrayBuffer()),
      );
    }
    return await verify({ directory, bundle: join(directory, "index.cjs") });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}
