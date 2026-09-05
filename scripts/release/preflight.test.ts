import { describe, expect, it } from "vitest";
import type { Request } from "./github.ts";
import { preflight } from "./preflight.ts";

describe("release preflight recovery", () => {
  const source = "a".repeat(40);
  function fixture(
    release: boolean | undefined,
    artifacts: { id: number; name: string; expired: boolean }[] = [],
  ) {
    const run = { event: "push", head_sha: source, path: ".github/workflows/release.yml" };
    const request = (async (path: string, method = "GET") => {
      expect(method).toBe("GET");
      if (path === "/actions/runs/123") return run;
      if (path === "/actions/runs/123/artifacts?per_page=100")
        return { artifacts, total_count: artifacts.length };
      if (path.startsWith("/releases?"))
        return release === undefined ? [] : [{ tag_name: "v0.1.0", draft: release }];
      throw new Error("Unexpected request " + path);
    }) as Request;
    return { request, run };
  }
  it("builds only when neither a release nor an original artifact exists", async () => {
    const f = fixture(undefined);
    await expect(preflight(f.request, "v0.1.0", source, "123")).resolves.toEqual({
      state: "build",
      artifactId: "",
    });
  });
  it.each([true, false])(
    "rejects an existing release without original bytes (draft=%s)",
    async (draft) => {
      const f = fixture(draft);
      await expect(preflight(f.request, "v0.1.0", source, "123")).rejects.toThrow(
        "without this run's original artifact",
      );
    },
  );
  it.each([undefined, true, false])(
    "routes saved bytes through verification (draft=%s)",
    async (draft) => {
      const f = fixture(draft, [{ id: 7, name: "release-v0.1.0", expired: false }]);
      await expect(preflight(f.request, "v0.1.0", source, "123")).resolves.toEqual({
        state: "reuse",
        artifactId: "7",
      });
    },
  );
  it.each(["expired", "duplicate", "other-tag", "wrong-source", "wrong-event", "wrong-workflow"])(
    "rejects terminal recovery with %s provenance",
    async (kind) => {
      const artifact = { id: 7, name: "release-v0.1.0", expired: kind === "expired" };
      const f = fixture(
        false,
        kind === "duplicate" ? [artifact, { ...artifact, id: 8 }] : [artifact],
      );
      if (kind === "other-tag") artifact.name = "release-v0.2.0";
      if (kind === "wrong-source") f.run.head_sha = "b".repeat(40);
      if (kind === "wrong-event") f.run.event = "workflow_dispatch";
      if (kind === "wrong-workflow") f.run.path = ".github/workflows/other.yml";
      await expect(preflight(f.request, "v0.1.0", source, "123")).rejects.toThrow(/.+/u);
    },
  );
});
