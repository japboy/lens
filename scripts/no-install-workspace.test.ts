import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";

function fixture(script: string, change?: "unexported" | "identity") {
  const root = mkdtempSync(join(tmpdir(), "lens-artifact-exports-"));
  const write = (path: string, source: string) => {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), source);
  };
  try {
    write("package.json", '{"type":"module"}');
    for (const file of ["no-install-workspace.ts", "workspace-policy.ts"])
      write(`scripts/${file}`, readFileSync(new URL(file, import.meta.url), "utf8"));
    for (const [name, facets] of [
      ["adapter-lit-prerenderer", ["verify", "source-snapshot"]],
      ["adapter-math-renderer", ["html-math-manifest"]],
    ] as const) {
      const exports = Object.fromEntries(
        facets.map((facet) => [`./${facet}`, `./public/${facet}.ts`]),
      );
      if (change === "unexported" && name === "adapter-lit-prerenderer") delete exports["./verify"];
      write(
        `packages/${name}/package.json`,
        JSON.stringify({
          name: change === "identity" && name === "adapter-lit-prerenderer" ? "other" : name,
          type: "module",
          exports,
        }),
      );
      for (const facet of facets)
        write(
          `packages/${name}/public/${facet}.ts`,
          `export const value = ${JSON.stringify(facet)};`,
        );
      write(`packages/${name}/internal.ts`, "export const privateValue = true;");
    }
    write(
      "apps/desktop/tooling/prerender/source.ts",
      `export { value as verify } from "adapter-lit-prerenderer/verify"; export { value as snapshot } from "adapter-lit-prerenderer/source-snapshot"; export { value as math } from "adapter-math-renderer/html-math-manifest";`,
    );
    write("outsider.ts", 'export * from "adapter-lit-prerenderer/verify";');
    write(
      "probe.mjs",
      `import {registerArtifactWorkspaceExports} from './scripts/no-install-workspace.ts';\nconst root = new URL('.',import.meta.url).pathname;\n${script}`,
    );
    return JSON.parse(
      execFileSync(process.execPath, ["probe.mjs"], { cwd: root, encoding: "utf8" }),
    );
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

describe("install-independent artifact public exports", () => {
  it("uses actual exports targets without installation and removes the hook afterward", () => {
    expect(
      fixture(
        `const hook=registerArtifactWorkspaceExports(root); let values; try {values=await import('./apps/desktop/tooling/prerender/source.ts');} finally {hook.deregister();} let after;try{await import('adapter-lit-prerenderer/source-snapshot')}catch(error){after=error.code;}console.log(JSON.stringify({values,after}));`,
      ),
    ).toEqual({
      values: { verify: "verify", snapshot: "source-snapshot", math: "html-math-manifest" },
      after: "ERR_MODULE_NOT_FOUND",
    });
  });
  it("respects unpublished facets and cleans up after a failed graph load", () => {
    expect(
      fixture(
        `const hook=registerArtifactWorkspaceExports(root);let failure;try{await import('./apps/desktop/tooling/prerender/source.ts')}catch(error){failure=error.code;}finally{hook.deregister();}let after;try{await import('adapter-lit-prerenderer/source-snapshot')}catch(error){after=error.code;}console.log(JSON.stringify({failure,after}));`,
        "unexported",
      ),
    ).toEqual({ failure: "ERR_PACKAGE_PATH_NOT_EXPORTED", after: "ERR_MODULE_NOT_FOUND" });
  });
  it("rejects a manifest identity mismatch before registering any hook", () => {
    expect(
      fixture(
        `let failure;try{registerArtifactWorkspaceExports(root)}catch(error){failure=error.message;}let after;try{await import('adapter-lit-prerenderer/source-snapshot')}catch(error){after=error.code;}console.log(JSON.stringify({failure,after}));`,
        "identity",
      ),
    ).toEqual({
      failure: "Artifact workspace package identity differs: adapter-lit-prerenderer",
      after: "ERR_MODULE_NOT_FOUND",
    });
  });
  it("leaves unrelated importers, private facets, and external dependencies to Node", () => {
    expect(
      fixture(
        `const hook=registerArtifactWorkspaceExports(root);const failures=[];try{for(const path of ['./outsider.ts','adapter-lit-prerenderer/internal','katex']){try{await import(path)}catch(error){failures.push(error.code)}}}finally{hook.deregister();}console.log(JSON.stringify(failures));`,
      ),
    ).toEqual(["ERR_MODULE_NOT_FOUND", "ERR_MODULE_NOT_FOUND", "ERR_MODULE_NOT_FOUND"]);
  });
});
