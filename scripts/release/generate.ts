import { GitHub, Manifest } from "release-please";
import { withReleaseProposalManifest } from "./proposal.ts";
import { github } from "./github.ts";
import { lifecycle } from "./lifecycle.ts";
import { commitSha, git } from "./source.ts";

export async function generate(
  root: string,
  token: string,
  repository: string,
  controllerSha: string,
  tag = "",
) {
  const [owner, repo] = repository.split("/");
  if (!owner || !repo || repository.split("/").length !== 2) throw new Error("Invalid repository");
  if (git(root, "rev-parse", "HEAD") !== commitSha(controllerSha))
    throw new Error("Controller checkout differs from workflow revision");
  const api = github(token, repository);
  const client = await GitHub.create({ owner, repo, token, defaultBranch: "main" });
  return lifecycle(
    api.request,
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
