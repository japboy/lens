import {
  appendFileSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  MACOS_BUILD_CACHE_CAPABILITY,
  MACOS_BUILD_CACHE_INPUTS,
  macosBuildContract,
  publishMacosBuildContract,
} from "./macos-build-contract.ts";
import { VERSION_FILES } from "./release/version.ts";

const root = fileURLToPath(new URL("..", import.meta.url));
const directories: string[] = [];
function fixture() {
  const directory = mkdtempSync(join(tmpdir(), "lens-macos-contract-"));
  directories.push(directory);
  for (const path of new Set([...MACOS_BUILD_CACHE_INPUTS, ...VERSION_FILES])) {
    mkdirSync(dirname(join(directory, path)), { recursive: true });
    copyFileSync(join(root, path), join(directory, path));
  }
  return directory;
}

afterEach(() => {
  for (const directory of directories.splice(0))
    rmSync(directory, { recursive: true, force: true });
  vi.restoreAllMocks();
});

describe("macOS dependency cache build contract", () => {
  it("is stable for the same inputs and covers every explicit capability input", () => {
    const directory = fixture();
    const baseline = macosBuildContract(directory);
    expect(baseline.deploymentTarget).toBe("15.2");
    expect(baseline.cacheKey).toMatch(new RegExp(`^${MACOS_BUILD_CACHE_CAPABILITY}-[a-f0-9]{64}$`));
    expect(macosBuildContract(directory)).toEqual(baseline);
    for (const path of MACOS_BUILD_CACHE_INPUTS) {
      const target = join(directory, path);
      const original = readFileSync(target);
      appendFileSync(target, "\n");
      expect(macosBuildContract(directory).cacheKey).not.toBe(baseline.cacheKey);
      writeFileSync(target, original);
    }
  });

  it("invalidates on virtual-root release profile changes even with identical dependency locks", () => {
    const directory = fixture();
    const baseline = macosBuildContract(directory).cacheKey;
    const cargo = join(directory, "Cargo.toml");
    const lock = readFileSync(join(directory, "Cargo.lock"), "utf8");
    writeFileSync(cargo, readFileSync(cargo, "utf8").replace("lto = true", "lto = false"));
    expect(macosBuildContract(directory).cacheKey).not.toBe(baseline);
    expect(readFileSync(join(directory, "Cargo.lock"), "utf8")).toBe(lock);
  });

  it("exports the validated deployment environment before cache restoration", () => {
    const directory = fixture();
    const output = join(directory, "output");
    const environment = join(directory, "environment");
    writeFileSync(output, "existing_output=preserved\n");
    writeFileSync(environment, "EXISTING=preserved\n");
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    publishMacosBuildContract(directory, { GITHUB_OUTPUT: output, GITHUB_ENV: environment });
    const { cacheKey } = macosBuildContract(directory);
    expect(readFileSync(output, "utf8")).toBe(
      `existing_output=preserved\ncache_key=${cacheKey}\ndeployment_target=15.2\n`,
    );
    expect(readFileSync(environment, "utf8")).toBe(
      "EXISTING=preserved\nMACOSX_DEPLOYMENT_TARGET=15.2\n",
    );
  });

  it("rejects a missing capability input and an unreviewed deployment contract", () => {
    const directory = fixture();
    const mac = join(directory, "apps/desktop/src-tauri/tauri.macos.conf.json");
    const original = readFileSync(mac, "utf8");
    writeFileSync(mac, original.replace("15.2", "15.3"));
    expect(() => macosBuildContract(directory)).toThrow("Unreviewed distribution");
    writeFileSync(mac, original);
    rmSync(join(directory, "scripts/macos-bundle-build.ts"));
    expect(() => macosBuildContract(directory)).toThrow("ENOENT");
  });
});
