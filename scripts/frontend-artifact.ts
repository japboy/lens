import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function frontendFiles(directory: string): Record<string, string> {
  const result: Record<string, string> = Object.create(null);
  const visit = (relative: string) => {
    for (const entry of readdirSync(join(directory, relative), { withFileTypes: true }).sort(
      (a, b) => a.name.localeCompare(b.name, "en"),
    )) {
      const path = relative ? `${relative}/${entry.name}` : entry.name;
      if (entry.isDirectory()) visit(path);
      else if (entry.isFile())
        result[path] = createHash("sha256")
          .update(readFileSync(join(directory, path)))
          .digest("hex");
      else throw new Error("Frontend artifacts must contain only regular files and directories");
    }
  };
  visit("");
  if (!result["index.html"]) throw new Error("Frontend entry asset is missing");
  return result;
}

export function frontendArtifact(mode: string, root: string): void {
  if (!["write", "check"].includes(mode))
    throw new Error("An explicit frontend artifact mode is required");
  if (
    execFileSync("git", ["status", "--porcelain", "-z", "--untracked-files=all"], { cwd: root })
      .length
  )
    throw new Error("Frontend artifact requires a clean source checkout");
  const source = execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
  const manifest = { version: 1, source, files: frontendFiles(join(root, "apps/desktop/dist")) };
  const destination = join(root, "target/ci/frontend-manifest.json");
  if (mode === "write") {
    mkdirSync(join(root, "target/ci"), { recursive: true });
    writeFileSync(destination, `${JSON.stringify(manifest, null, 2)}\n`);
  } else if (
    JSON.stringify(JSON.parse(readFileSync(destination, "utf8"))) !== JSON.stringify(manifest)
  )
    throw new Error(
      "Frontend artifact source, file set or digest does not match the tested checkout",
    );
  process.stdout.write(
    `${JSON.stringify({ mode, source, files: Object.keys(manifest.files).length })}\n`,
  );
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  if (process.argv.length !== 3) throw new Error("Exactly one artifact mode is required");
  frontendArtifact(process.argv[2]!, fileURLToPath(new URL("..", import.meta.url)));
}
