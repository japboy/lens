import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { refreshCargoLock } from "./cargo-lock.ts";

const repositoryLock = readFileSync(new URL("../../Cargo.lock", import.meta.url), "utf8");
const registryVersion = /name = "serde"\nversion = "([^"]+)"\nsource = "registry\+/u.exec(
  repositoryLock,
)![1]!;
const directories: string[] = [];
let cacheSeeded = false;
const run = (root: string, args: string[], offline = true) =>
  execFileSync("cargo", [...args, ...(offline ? ["--offline"] : [])], {
    cwd: root,
    encoding: "utf8",
    timeout: 120_000,
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });

function fixture(version: string) {
  const root = mkdtempSync(join(tmpdir(), "lens-cargo-lock-"));
  directories.push(root);
  const baseline = join(root, "baseline");
  const candidate = join(root, "candidate");
  for (const name of ["app", "local-serde"]) {
    mkdirSync(join(baseline, name, "src"), { recursive: true });
    writeFileSync(join(baseline, name, "src/lib.rs"), "");
  }
  writeFileSync(
    join(baseline, "Cargo.toml"),
    `[workspace]\nmembers = ["app", "local-serde"]\nresolver = "2"\n[workspace.package]\nversion = "${version}"\n`,
  );
  writeFileSync(
    join(baseline, "app/Cargo.toml"),
    `[package]\nname = "collision-app"\nversion.workspace = true\nedition = "2021"\n[dependencies]\nlocal_serde = { package = "serde", path = "../local-serde" }\nregistry_serde = { package = "serde", version = "=${registryVersion}" }\n`,
  );
  writeFileSync(
    join(baseline, "local-serde/Cargo.toml"),
    '[package]\nname = "serde"\nversion.workspace = true\nedition = "2021"\n',
  );
  if (cacheSeeded) run(baseline, ["generate-lockfile"]);
  else {
    // Fetch this fixture's exact registry package before offline repair proofs on clean CI.
    run(baseline, ["metadata", "--all-features", "--format-version", "1"], false);
    cacheSeeded = true;
  }
  cpSync(baseline, candidate, { recursive: true });
  return { baseline, candidate, lock: readFileSync(join(baseline, "Cargo.lock"), "utf8") };
}

function bump(root: string, version: string) {
  const path = join(root, "Cargo.toml");
  writeFileSync(
    path,
    readFileSync(path, "utf8").replace(/^version = "[^"]+"$/mu, `version = "${version}"`),
  );
}

const sourced = (lock: string) =>
  lock
    .split("[[package]]")
    .filter((entry) => /^source = /mu.test(entry))
    .sort();

afterEach(() => {
  for (const path of directories.splice(0)) rmSync(path, { recursive: true, force: true });
});

describe("Cargo-owned workspace lock refresh", () => {
  const nextRegistryVersion = registryVersion.replace(/\d+$/u, (patch) =>
    String(Number(patch) + 1),
  );
  it.each([
    ["different-version names", "0.2.0", "0.3.0"],
    ["equal-version names", registryVersion, nextRegistryVersion],
    ["bump into a registry version collision", "0.2.0", registryVersion],
  ])(
    "repairs real references for %s and preserves external packages",
    (_name, before, after) => {
      const { baseline, candidate, lock } = fixture(before);
      bump(candidate, after);
      // Reproduce the former package-version-only updater, leaving dependency IDs stale.
      const partial = lock
        .split("[[package]]")
        .map((entry) =>
          /^source = /mu.test(entry)
            ? entry
            : entry.replace(/^version = "[^"]+"$/mu, `version = "${after}"`),
        )
        .join("[[package]]");
      writeFileSync(join(candidate, "Cargo.lock"), partial);
      expect(() => run(candidate, ["metadata", "--locked", "--format-version", "1"])).toThrow(
        /--locked/u,
      );
      // The production candidate retains the trusted lock until Cargo refreshes it.
      writeFileSync(join(candidate, "Cargo.lock"), lock);
      const repaired = refreshCargoLock(baseline, candidate, true);
      expect(
        run(candidate, ["metadata", "--locked", "--all-features", "--format-version", "1"]),
      ).toContain('"resolve":');
      expect(repaired).toBe(readFileSync(join(candidate, "Cargo.lock"), "utf8"));
      expect(readFileSync(join(baseline, "Cargo.lock"), "utf8")).toBe(lock);
      expect(sourced(repaired)).toEqual(sourced(lock));
      expect(repaired).toContain(` "serde ${after}"`);
      expect(
        repaired.includes(
          ` "serde ${registryVersion} (registry+https://github.com/rust-lang/crates.io-index)"`,
        ),
      ).toBe(after === registryVersion);
    },
    120_000,
  );

  it("rejects changed external dependency features", () => {
    const { baseline, candidate } = fixture("0.2.0");
    bump(candidate, "0.3.0");
    const path = join(candidate, "app/Cargo.toml");
    writeFileSync(
      path,
      readFileSync(path, "utf8").replace(
        `version = "=${registryVersion}"`,
        `version = "=${registryVersion}", features = ["rc"]`,
      ),
    );
    expect(() => refreshCargoLock(baseline, candidate, true)).toThrow(
      "changed dependency resolution",
    );
  }, 120_000);

  it("rejects a sourced checksum mismatch", () => {
    const { baseline, candidate, lock } = fixture("0.2.0");
    bump(candidate, "0.3.0");
    writeFileSync(
      join(baseline, "Cargo.lock"),
      lock.replace(/^checksum = "[^"]+"$/mu, `checksum = "${"0".repeat(64)}"`),
    );
    expect(() => refreshCargoLock(baseline, candidate, true)).toThrow(/checksum/iu);
  }, 120_000);

  it("requires a separate candidate", () => {
    expect(() => refreshCargoLock(tmpdir(), tmpdir(), true)).toThrow("isolated candidate");
  });
});
