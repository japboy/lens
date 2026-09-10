import { readFileSync } from "node:fs";
import { join } from "node:path";
import { MEMBERS } from "../workspace-policy.ts";

const cargoMembers = MEMBERS.filter((member) => member.ecosystem === "cargo");
const pnpmFiles = MEMBERS.filter((member) => member.ecosystem === "pnpm").map((member) =>
  join(member.directory, "package.json"),
);
export const VERSION_FILES = [
  "apps/desktop/src-tauri/tauri.conf.json",
  ...pnpmFiles,
  "Cargo.toml",
  ...cargoMembers.map((member) => join(member.directory, "Cargo.toml")),
  "Cargo.lock",
  ".release-please-manifest.json",
] as const;
export const STABLE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u;

export function stableVersion(value: unknown): string {
  if (typeof value !== "string" || !STABLE_VERSION.test(value))
    throw new Error("Expected a stable X.Y.Z application version");
  return value;
}

export function tagVersion(tag: string): string {
  if (!tag.startsWith("v")) throw new Error("Expected an annotated vX.Y.Z release tag");
  return stableVersion(tag.slice(1));
}

// Owned TOML fields use explicit declarations. Cargo remains the TOML/resolution
// authority through metadata --locked and compilation in workspace verification.
function section(content: string, name: string): string {
  const parts = content.split(`[${name}]`);
  if (parts.length !== 2) throw new Error(`Expected exactly one TOML [${name}] section`);
  return parts[1]!.split(/^\[/mu)[0]!;
}

function declaration(content: string, field: string): string {
  const values = [...content.matchAll(new RegExp(`^${field} = "([^"\\n]+)"$`, "gmu"))];
  if (values.length !== 1) throw new Error(`Expected exactly one explicit TOML ${field}`);
  return values[0]![1]!;
}

// Release Please owns manifests; Cargo resolves the lock in the following step.
export function manifestVersionState(files: Readonly<Record<string, string>>): {
  version: string;
  bootstrapped: boolean;
} {
  const json = (path: string) => JSON.parse(files[path]!);
  const version = stableVersion(json("apps/desktop/src-tauri/tauri.conf.json").version);
  const manifest = json(".release-please-manifest.json");
  const keys = Object.keys(manifest);
  if (keys.length > 1 || (keys.length === 1 && keys[0] !== "."))
    throw new Error("Only one root release component is allowed");
  const mirrors: unknown[] = [
    ...pnpmFiles.map((path) => json(path).version),
    declaration(section(files["Cargo.toml"]!, "workspace.package"), "version"),
    ...(keys.length ? [manifest["."]] : []),
  ];
  for (const member of cargoMembers) {
    const path = join(member.directory, "Cargo.toml");
    const content = section(files[path]!, "package");
    if (
      declaration(content, "name") !== member.name ||
      content.match(/^version[^\n]*$/gmu)?.join("\n") !== "version.workspace = true"
    )
      throw new Error(
        `${path}: expected the declared package identity and inherited workspace version`,
      );
  }
  if (mirrors.some((value) => stableVersion(value) !== version))
    throw new Error("Workspace version mirrors disagree with Tauri authority");
  if (!keys.length && version !== "0.1.0")
    throw new Error("Only initial 0.1.0 may precede the first release PR");
  return { version, bootstrapped: keys.length === 1 };
}

export function versionState(files: Readonly<Record<string, string>>): {
  version: string;
  bootstrapped: boolean;
} {
  const state = manifestVersionState(files);
  const entries = files["Cargo.lock"]!.split(/^\[\[package\]\]\s*$/mu).slice(1);
  const local = entries.filter((entry) => !/^source\s*=/mu.test(entry));
  if (local.length !== cargoMembers.length)
    throw new Error("Expected exactly the workspace packages in local Cargo.lock entries");
  for (const member of cargoMembers) {
    const matching = local.filter((entry) => declaration(entry, "name") === member.name);
    if (matching.length !== 1)
      throw new Error(`Expected exactly one local ${member.name} Cargo.lock entry`);
    if (stableVersion(declaration(matching[0]!, "version")) !== state.version)
      throw new Error("Workspace version mirrors disagree with Tauri authority");
  }
  return state;
}

export function readVersion(root: string) {
  return versionState(
    Object.fromEntries(VERSION_FILES.map((path) => [path, readFileSync(join(root, path), "utf8")])),
  );
}

export function compareVersions(left: string, right: string): number {
  const a = stableVersion(left).split(".").map(BigInt);
  const b = stableVersion(right).split(".").map(BigInt);
  for (let index = 0; index < 3; index++)
    if (a[index] !== b[index]) return a[index]! < b[index]! ? -1 : 1;
  return 0;
}
