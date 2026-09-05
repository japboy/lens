import { readFileSync } from "node:fs";
import { join } from "node:path";

export const VERSION_FILES = [
  "apps/desktop/src-tauri/tauri.conf.json",
  "package.json",
  "apps/desktop/package.json",
  "apps/desktop/src-tauri/Cargo.toml",
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

// These two owned TOML fields deliberately admit only explicit string declarations.
// Cargo remains the TOML/resolution authority through metadata --locked and compilation.
function declaration(section: string, field: string): string {
  const values = [...section.matchAll(new RegExp(`^${field} = "([^"\\n]+)"$`, "gmu"))];
  if (values.length !== 1) throw new Error(`Expected exactly one explicit TOML ${field}`);
  return values[0]![1]!;
}

export function versionState(files: Readonly<Record<string, string>>): {
  version: string;
  bootstrapped: boolean;
} {
  const json = (path: string) => JSON.parse(files[path]!);
  const version = stableVersion(json(VERSION_FILES[0]).version);
  const manifest = json(VERSION_FILES[5]);
  const keys = Object.keys(manifest);
  if (keys.length > 1 || (keys.length === 1 && keys[0] !== "."))
    throw new Error("Only one root release component is allowed");
  const cargo = files[VERSION_FILES[3]]!.split(/^\[package\]\s*$/mu);
  if (cargo.length !== 2) throw new Error("Expected one application Cargo package");
  const cargoVersion = declaration(cargo[1]!.split(/^\[/mu)[0]!, "version");
  const entries = files[VERSION_FILES[4]]!.split(/^\[\[package\]\]\s*$/mu).slice(1);
  const desktop = entries.filter((entry) => /^name = "desktop"$/mu.test(entry));
  if (desktop.length !== 1 || /^source\s*=/mu.test(desktop[0]!))
    throw new Error("Expected exactly one local desktop Cargo.lock entry");
  const mirrors: unknown[] = [
    json(VERSION_FILES[1]).version,
    json(VERSION_FILES[2]).version,
    cargoVersion,
    declaration(desktop[0]!, "version"),
    ...(keys.length ? [manifest["."]] : []),
  ];
  if (mirrors.some((value) => stableVersion(value) !== version))
    throw new Error("Application version mirrors disagree with Tauri authority");
  if (!keys.length && version !== "0.1.0")
    throw new Error("Only initial 0.1.0 may precede the first release PR");
  return { version, bootstrapped: keys.length === 1 };
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
