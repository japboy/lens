import { GitHub, Manifest } from "release-please";
import { withReleaseProposalManifest } from "./proposal.ts";
import { github, type Request } from "./github.ts";
import { lifecycle } from "./lifecycle.ts";
import { commitSha, git } from "./source.ts";

export function policyReader(request: Request): <T>(path: string) => Promise<T> {
  return <T>(path: string) => {
    if (path !== "/rules/branches/main" && !/^\/rulesets\/[1-9]\d*$/u.test(path))
      throw new Error("Invalid release policy endpoint");
    return request<T>(path, "GET");
  };
}

export async function generate(
  root: string,
  token: string,
  policyToken: string,
  repository: string,
  controllerSha: string,
  tag = "",
) {
  const [owner, repo] = repository.split("/");
  if (!owner || !repo || repository.split("/").length !== 2) throw new Error("Invalid repository");
  if (git(root, "rev-parse", "HEAD") !== commitSha(controllerSha))
    throw new Error("Controller checkout differs from workflow revision");
  if (!policyToken) throw new Error("Missing release policy token");
  const api = github(token, repository);
  const policy = github(policyToken, repository);
  const client = await GitHub.create({ owner, repo, token, defaultBranch: "main" });
  return lifecycle(
    api.request,
    policyReader(policy.request),
    root,
    repository,
    controllerSha,
    {
      async createReleases() {
        const manifest = await Manifest.fromManifest(client, "main");
        return manifest.createReleases();
      },
      async propose() {
        return withReleaseProposalManifest(
          { github: client, root, baseSha: controllerSha },
          (manifest) => manifest.createPullRequests(),
        );
      },
    },
    tag,
  );
}
