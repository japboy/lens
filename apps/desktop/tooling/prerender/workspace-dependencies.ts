import { mkdir, readFile, realpath, symlink } from "node:fs/promises";
import { dirname, join, relative } from "node:path";
import { WORKSPACE_PACKAGE_PATHS } from "./source.ts";

/** Local dependencies resolve only to the sealed copy. Only installed third-party
 * dependency directories may point outside the generation transaction. */
export async function linkGenerationDependencies(
  repository: string,
  staging: string,
): Promise<void> {
  const directories = [".", "apps/desktop", ...WORKSPACE_PACKAGE_PATHS];
  const manifests = await Promise.all(
    directories.map(async (directory) => ({
      directory,
      manifest: JSON.parse(await readFile(join(staging, directory, "package.json"), "utf8")) as {
        name: string;
        dependencies?: Record<string, string>;
        devDependencies?: Record<string, string>;
      },
    })),
  );
  const installedStore = await realpath(join(repository, "node_modules/.pnpm"));
  const local = new Map(manifests.map(({ directory, manifest }) => [manifest.name, directory]));
  for (const { directory, manifest } of manifests) {
    const dependencies = { ...manifest.dependencies, ...manifest.devDependencies };
    for (const [name, version] of Object.entries(dependencies)) {
      const member = local.get(name);
      let target: string;
      if (member !== undefined) {
        if (version !== "workspace:*") throw new Error(`Unsealed local dependency: ${name}`);
        target = join(staging, member);
      } else {
        if (/^(?:workspace:|file:|link:)/u.test(version))
          throw new Error(`Unknown local dependency: ${name}`);
        target = await realpath(join(repository, directory, "node_modules", name));
        const path = relative(installedStore, target).replaceAll("\\", "/");
        if (
          path === ".." ||
          path.startsWith("../") ||
          path.startsWith("/") ||
          !path.includes("/node_modules/")
        )
          throw new Error(`Installed dependency escapes sealed workspace: ${name}`);
      }
      const destination = join(staging, directory, "node_modules", name);
      await mkdir(dirname(destination), { recursive: true });
      await symlink(target, destination, "dir");
    }
  }
}
