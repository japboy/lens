import { readFileSync } from "node:fs";
import { registerHooks } from "node:module";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { MEMBERS } from "./workspace-policy.ts";

// Only the dependency-free artifact admission graph needs source resolution
// before installation. Node owns package exports and self-reference semantics.
export const ARTIFACT_WORKSPACE_FACETS = [
  "adapter-lit-prerenderer/verify",
  "adapter-lit-prerenderer/source-snapshot",
  "adapter-math-renderer/html-math-manifest",
] as const;

export function registerArtifactWorkspaceExports(repository: string) {
  const packages = new Map<string, string>();
  for (const facet of ARTIFACT_WORKSPACE_FACETS) {
    const name = facet.split("/")[0]!;
    if (packages.has(name)) continue;
    const member = MEMBERS.find((entry) => entry.ecosystem === "pnpm" && entry.name === name);
    if (!member) throw new Error(`Missing artifact workspace member: ${name}`);
    const manifest = join(repository, member.directory, "package.json");
    if (JSON.parse(readFileSync(manifest, "utf8")).name !== name)
      throw new Error(`Artifact workspace package identity differs: ${name}`);
    packages.set(name, pathToFileURL(manifest).href);
  }
  const callers = new Set(
    ["apps/desktop/tooling/prerender/source.ts", "apps/desktop/tooling/prerender/verify.ts"].map(
      (path) => pathToFileURL(join(repository, path)).href,
    ),
  );
  return registerHooks({
    resolve(specifier, context, nextResolve) {
      if (
        !context.parentURL ||
        !callers.has(context.parentURL) ||
        !(ARTIFACT_WORKSPACE_FACETS as readonly string[]).includes(specifier)
      )
        return nextResolve(specifier, context);
      return nextResolve(specifier, {
        ...context,
        parentURL: packages.get(specifier.split("/")[0]!)!,
      });
    },
  });
}
