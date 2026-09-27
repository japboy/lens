import { parse } from "@iarna/toml";

export function parseToml(content: string, path: string): Record<string, unknown> {
  try {
    return parse(content);
  } catch (cause) {
    throw new Error(`${path}: invalid TOML`, { cause });
  }
}

export function tomlTable(value: unknown, path: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) || value instanceof Date)
    throw new Error(`${path}: expected a TOML table`);
  return value as Record<string, unknown>;
}

type LockPackage = {
  name: string;
  version: string;
  source?: string;
  checksum?: string;
};

export function cargoLockPackages(content: string, path = "Cargo.lock"): LockPackage[] {
  const packages = parseToml(content, path).package;
  if (!Array.isArray(packages)) throw new Error(`${path}: expected package entries`);
  return packages.map((value) => {
    const entry = tomlTable(value, `${path}: package`);
    for (const key of ["name", "version"])
      if (typeof entry[key] !== "string" || !entry[key])
        throw new Error(`${path}: expected a nonempty package ${key}`);
    for (const key of ["source", "checksum"])
      if (entry[key] !== undefined && (typeof entry[key] !== "string" || !entry[key]))
        throw new Error(`${path}: expected a nonempty package ${key}`);
    return entry as LockPackage;
  });
}
