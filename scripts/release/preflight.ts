import { pages } from "./github.ts";
import type { Request } from "./github.ts";
import type { Release } from "./control.ts";

// Published releases also require the original artifact and the full verification path.
export async function preflight(
  request: Request,
  tag: string,
  source: string,
  runId: string,
): Promise<{ state: "build" | "reuse"; artifactId: string }> {
  const run = await request<{ event: string; head_sha: string; path: string }>(
    `/actions/runs/${runId}`,
  );
  if (
    run.event !== "push" ||
    run.head_sha !== source ||
    run.path !== ".github/workflows/release.yml"
  )
    throw new Error("Unexpected release workflow provenance");
  const artifacts = await request<{
    artifacts: { id: number; name: string; expired: boolean }[];
    total_count: number;
  }>(`/actions/runs/${runId}/artifacts?per_page=100`);
  if (artifacts.total_count > 100) throw new Error("Unexpected artifact count");
  const saved = artifacts.artifacts.filter((artifact) => artifact.name === `release-${tag}`);
  if (saved.length > 1 || saved.some((artifact) => artifact.expired))
    throw new Error("Conflicting or expired original release artifact; use a new version");
  const releases = (await pages<Release>(request, "/releases")).filter(
    (item) => item.tag_name === tag,
  );
  if (!saved.length && releases.length)
    throw new Error("Release exists without this run's original artifact");
  return {
    state: saved.length ? "reuse" : "build",
    artifactId: saved[0] ? String(saved[0].id) : "",
  };
}
