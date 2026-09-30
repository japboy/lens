import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "@iarna/toml";

type Identity = readonly [version: string, digest: string];
type Read = (path: string) => string | null;
const nodeHistoryPath = "apps/desktop/src-tauri/agent-runtime/node-history.toml";
const table = (text: string) => parse(text) as Record<string, unknown>;
const textField = (entry: Record<string, unknown>, key: string): string => {
  if (!entry || typeof entry[key] !== "string") throw new Error(`Missing ${key}`);
  return entry[key];
};
function previous(value: unknown, digestKey: string): Identity[] {
  if (!Array.isArray(value)) throw new Error("History must be an array");
  return value.map((entry: Record<string, unknown>) => [
    textField(entry, "version"),
    textField(entry, digestKey),
  ]);
}
export function bootstrapIdentities(read: Read): { node: Identity[]; pnpm: Identity[] } {
  const config = table(read("mise.toml") ?? "");
  const lock = table(read("mise.lock") ?? "");
  // Artifact formats are validated by the required Rust build. This gate compares identities.
  const current = (tool: string): Identity => {
    const version = textField(config.tools as Record<string, unknown>, tool);
    const entries = (lock.tools as Record<string, unknown>)[tool] as Record<string, unknown>[];
    if (!Array.isArray(entries) || entries.length !== 1 || entries[0]?.version !== version)
      throw new Error(`${tool} lock must identify its declared version`);
    const artifact = entries[0]["platforms.macos-arm64"] as Record<string, unknown>;
    return [version, textField(artifact, "checksum")];
  };
  const [nodeVersion, nodeDigest] = current("node");
  const nodeHistory = read(nodeHistoryPath);
  const node: Identity[] = [
    [nodeVersion, nodeDigest.replace(/^sha256:/u, "")],
    ...(nodeHistory ? previous(table(nodeHistory).previous, "archive_sha256") : []),
  ];
  // main before managed pnpm policy shipped the JavaScript SHA512 identity in Rust.
  const source = read("apps/desktop/src-tauri/src/agent_runtime.rs") ?? "";
  const legacyVersion = /const PNPM_VERSION: &str = "([^"]+)";/u.exec(source)?.[1];
  const legacyDigest = /const PNPM_ARCHIVE_SHA512: &str = "([a-f0-9]{128})";/u.exec(source)?.[1];
  let pnpm: Identity[];
  if (legacyVersion && legacyDigest) {
    pnpm = [[legacyVersion, `sha512:${legacyDigest}`]];
  } else {
    const history = read("apps/desktop/src-tauri/agent-runtime/pnpm-history.toml");
    if (!history) throw new Error("Explicit pnpm history required");
    pnpm = [current("pnpm"), ...previous(table(history).previous, "archive_digest")];
  }
  return { node, pnpm };
}
export function assertPreservedBootstrapIdentities(
  before: ReturnType<typeof bootstrapIdentities>,
  after: ReturnType<typeof bootstrapIdentities>,
): void {
  for (const tool of ["node", "pnpm"] as const) {
    for (const [version, digest] of before[tool]) {
      if (
        !after[tool].some(
          ([nextVersion, nextDigest]) => version === nextVersion && digest === nextDigest,
        )
      )
        throw new Error(
          `${tool} ${version}: preserve the exact old bootstrap identity in approved history`,
        );
    }
  }
}
export function verifyBootstrapHistory(root: string, base: string): void {
  if (!/^[a-f0-9]{40}$/u.test(base)) throw new Error("Exact base commit SHA is required");
  const git = (args: string[]) =>
    execFileSync("git", ["--literal-pathspecs", ...args], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 2 * 1024 * 1024,
    });
  if (git(["rev-parse", "--verify", `${base}^{commit}`]).trim() !== base)
    throw new Error("Unexpected base commit");
  const before = bootstrapIdentities((path) => {
    if (!git(["ls-tree", base, "--", path])) return null;
    return git(["show", `${base}:${path}`]);
  });
  const after = bootstrapIdentities((path) => {
    try {
      return readFileSync(resolve(root, path), "utf8");
    } catch {
      return null;
    }
  });
  assertPreservedBootstrapIdentities(before, after);
}
if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
  verifyBootstrapHistory(fileURLToPath(new URL("..", import.meta.url)), process.env.BASE_SHA ?? "");
  process.stdout.write(
    "Current and all previously approved bootstrap identities remain accepted.\n",
  );
}
