import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "@iarna/toml";

type Identity = readonly [version: string, digest: string];
type Read = (path: string) => string | null;
const pnpmPath = "apps/desktop/src-tauri/agent-runtime/pnpm.toml";
const nodeHistoryPath = "apps/desktop/src-tauri/agent-runtime/node-history.toml";
const table = (text: string) => parse(text) as Record<string, unknown>;
const textField = (entry: Record<string, unknown>, key: string): string => {
  if (typeof entry[key] !== "string") throw new Error(`Missing ${key}`);
  return entry[key];
};
function previous(value: unknown, digestKey: string): Identity[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new Error("History must be an array");
  return value.map((entry: Record<string, unknown>) => [
    textField(entry, "version"),
    textField(entry, digestKey),
  ]);
}
export function bootstrapIdentities(read: Read): { node: Identity[]; pnpm: Identity[] } {
  const config = table(read("mise.toml") ?? "");
  const version = textField(config.tools as Record<string, unknown>, "node");
  const lock = table(read("mise.lock") ?? "");
  const entries = (lock.tools as Record<string, unknown>).node as Record<string, unknown>[];
  if (!Array.isArray(entries) || entries.length !== 1 || entries[0]?.version !== version)
    throw new Error("Node lock must match its exact declaration");
  const artifact = entries[0]["platforms.macos-arm64"] as Record<string, unknown>;
  const checksum = textField(artifact, "checksum");
  if (!/^sha256:[a-f0-9]{64}$/u.test(checksum)) throw new Error("Node SHA256 required");
  const history = read(nodeHistoryPath);
  const node: Identity[] = [
    [version, checksum.slice(7)],
    ...previous(history ? table(history).previous : undefined, "archive_sha256"),
  ];
  const declaration = read(pnpmPath);
  let pnpm: Identity[];
  if (declaration) {
    const policy = table(declaration);
    pnpm = [
      [textField(policy, "version"), textField(policy, "integrity")],
      ...previous(policy.previous, "integrity"),
    ];
  } else {
    // Migration from the previously compiled pnpm declaration, never from installed records.
    const source = read("apps/desktop/src-tauri/src/agent_runtime.rs") ?? "";
    const version = /const PNPM_VERSION: &str = "([^"]+)";/u.exec(source)?.[1];
    const digest = /const PNPM_ARCHIVE_SHA512: &str = "([a-f0-9]{128})";/u.exec(source)?.[1];
    if (!version || !digest) throw new Error("Cannot resolve base pnpm identity");
    pnpm = [[version, `sha512-${Buffer.from(digest, "hex").toString("base64")}`]];
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
