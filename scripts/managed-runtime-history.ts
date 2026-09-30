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
  const source = read("apps/desktop/src-tauri/src/agent_runtime.rs") ?? "";
  const legacyVersion = /const PNPM_VERSION: &str = "([^"]+)";/u.exec(source)?.[1];
  const legacyDigest = /const PNPM_ARCHIVE_SHA512: &str = "([a-f0-9]{128})";/u.exec(source)?.[1];
  let pnpm: Identity[];
  if (declaration) {
    const policy = table(declaration);
    const normalize = ([version, integrity]: Identity): Identity => {
      const bytes = Buffer.from(integrity.replace(/^sha512-/u, ""), "base64");
      if (
        !integrity.startsWith("sha512-") ||
        bytes.length !== 64 ||
        `sha512-${bytes.toString("base64")}` !== integrity
      )
        throw new Error("Canonical historical pnpm SHA512 SRI required");
      return [version, `sha512:${bytes.toString("hex")}`];
    };
    pnpm = [
      [textField(policy, "version"), textField(policy, "integrity")] as Identity,
      ...previous(policy.previous, "integrity"),
    ].map(normalize);
  } else if (legacyVersion && legacyDigest) {
    // The pre-policy runtime was independent of development's native pnpm.
    pnpm = [[legacyVersion, `sha512:${legacyDigest}`]];
  } else {
    const pnpmVersion = textField(config.tools as Record<string, unknown>, "pnpm");
    const pnpmEntries = (lock.tools as Record<string, unknown>).pnpm as Record<string, unknown>[];
    if (
      !/^12\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/u.test(pnpmVersion) ||
      !Array.isArray(pnpmEntries) ||
      pnpmEntries.length !== 1 ||
      pnpmEntries[0]?.version !== pnpmVersion ||
      pnpmEntries[0]?.backend !== "aqua:pnpm/pnpm"
    )
      throw new Error("Native pnpm lock must match its exact development declaration");
    const artifact = pnpmEntries[0]["platforms.macos-arm64"] as Record<string, unknown>;
    const checksum = textField(artifact, "checksum");
    if (
      artifact.url !==
        `https://github.com/pnpm/pnpm/releases/download/v${pnpmVersion}/pnpm-darwin-arm64.tar.gz` ||
      !/^sha256:[a-f0-9]{64}$/u.test(checksum)
    )
      throw new Error("Official native pnpm archive and SHA256 required");
    const history = read("apps/desktop/src-tauri/agent-runtime/pnpm-history.toml");
    if (!history) throw new Error("Explicit pnpm history required");
    const policy = table(history);
    if (Object.keys(policy).some((key) => key !== "previous"))
      throw new Error("pnpm history owns only previous identities");
    if (
      !Array.isArray(policy.previous) ||
      policy.previous.some(
        (entry) =>
          typeof entry !== "object" ||
          entry === null ||
          Object.keys(entry).length !== 2 ||
          Object.keys(entry).some((key) => !["version", "archive_digest"].includes(key)),
      )
    )
      throw new Error("Explicit pnpm previous identities require version and archive_digest only");
    const old = previous(policy.previous, "archive_digest");
    const seen = new Set<string>();
    for (const [version, digest] of old) {
      if (
        !/^(11|12)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/u.test(version) ||
        !(version.startsWith("11.") ? /^sha512:[a-f0-9]{128}$/u : /^sha256:[a-f0-9]{64}$/u).test(
          digest,
        ) ||
        seen.has(version) ||
        (version === pnpmVersion && digest !== checksum)
      )
        throw new Error("Noncanonical or conflicting historical pnpm identity");
      seen.add(version);
    }
    pnpm = [[pnpmVersion, checksum], ...old];
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
