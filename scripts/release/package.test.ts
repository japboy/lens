import * as childProcess from "node:child_process";
import * as fs from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CONFIGURATION_FILES, sha256, verifyArtifact } from "./artifact.ts";
import { RELEASE_WORKFLOW, verifyArtifactV2 } from "./receipt.ts";
import { VERSION_FILES, readVersion } from "./version.ts";

vi.mock("node:fs", async (original) => {
  const actual = await original<typeof fs>();
  return {
    ...actual,
    writeFileSync: vi.fn<typeof fs.writeFileSync>(actual.writeFileSync),
    readFileSync: vi.fn<typeof fs.readFileSync>(actual.readFileSync),
  };
});
vi.mock("node:child_process", async (original) => {
  const actual = await original<typeof childProcess>();
  return { ...actual, execFileSync: vi.fn<typeof childProcess.execFileSync>(actual.execFileSync) };
});
const root = fileURLToPath(new URL("../../", import.meta.url));
const temporaries: string[] = [];
const argv = process.argv;
afterEach(() => {
  process.argv = argv;
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  for (const directory of temporaries.splice(0))
    fs.rmSync(directory, { recursive: true, force: true });
});
function fixture() {
  const directory = fs.mkdtempSync(join(tmpdir(), "lens-package-"));
  temporaries.push(directory);
  const git = (...args: string[]) =>
    childProcess
      .execFileSync("git", ["-c", "core.hooksPath=/dev/null", ...args], {
        cwd: directory,
        encoding: "utf8",
        env: {
          ...process.env,
          GIT_AUTHOR_NAME: "Fixture",
          GIT_AUTHOR_EMAIL: "fixture@example.test",
          GIT_COMMITTER_NAME: "Fixture",
          GIT_COMMITTER_EMAIL: "fixture@example.test",
        },
      })
      .trim();
  for (const path of new Set([...VERSION_FILES, ...CONFIGURATION_FILES, "CHANGELOG.md"])) {
    fs.mkdirSync(dirname(join(directory, path)), { recursive: true });
    fs.copyFileSync(join(root, path), join(directory, path));
  }
  fs.writeFileSync(join(directory, ".gitignore"), "target/\n");
  git("init", "--quiet");
  git("add", ".");
  git("-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixture");
  const source = git("rev-parse", "HEAD");
  const { version } = readVersion(directory);
  const name = `Lens_${version}_aarch64.dmg`;
  const dmgPath = join(directory, "target/aarch64-apple-darwin/release/bundle/dmg", name);
  fs.mkdirSync(dirname(dmgPath), { recursive: true });
  const dmg = Buffer.from("native verification belongs to the preceding build gate");
  fs.writeFileSync(dmgPath, dmg);
  return {
    directory,
    source,
    version,
    name,
    dmg,
    dmgPath,
    destination: join(directory, "target/release-artifact"),
  };
}

describe("production package CLI", () => {
  it("writes schema 2 directly and verifies the final four files once", async () => {
    const f = fixture();
    const controller = childProcess
      .execFileSync("git", ["rev-parse", "HEAD"], { cwd: root, encoding: "utf8" })
      .trim();
    const actual = await vi.importActual<typeof childProcess>("node:child_process");
    vi.mocked(childProcess.execFileSync).mockImplementation(((
      file: string,
      args: string[],
      options: object,
    ) => {
      if (["pnpm", "rustc", "xcodebuild", "xcrun", "sw_vers"].includes(file))
        return "fixture tool version";
      return actual.execFileSync(file, args, options);
    }) as typeof childProcess.execFileSync);
    for (const [key, value] of Object.entries({
      CONTROLLER_SHA: controller,
      GITHUB_SHA: controller,
      SOURCE_ROOT: f.directory,
      SOURCE_SHA: f.source,
      RELEASE_TAG: `v${f.version}`,
      GITHUB_REPOSITORY: "owner/repo",
      GITHUB_RUN_ID: "123",
      GITHUB_RUN_ATTEMPT: "2",
      PREVIOUS_TAG: "",
    }))
      vi.stubEnv(key, value);
    vi.mocked(fs.writeFileSync).mockClear();
    vi.mocked(fs.readFileSync).mockClear();
    process.argv = [process.execPath, join(root, "scripts/release/cli.ts"), "package"];
    await import("./cli.ts");

    const manifestPath = join(f.destination, "release-manifest.json");
    const manifests = vi
      .mocked(fs.writeFileSync)
      .mock.calls.filter(([path]) => path === manifestPath);
    expect(manifests).toHaveLength(1);
    expect(JSON.parse(String(manifests[0]![1]))).toMatchObject({
      schema: 2,
      source: f.source,
      controller,
      workflow: RELEASE_WORKFLOW,
      runId: "123",
      runAttempt: "2",
      verificationAttempt: "2",
    });
    // One source read constructs the digest, and one destination read verifies the final artifact.
    expect(
      vi.mocked(fs.readFileSync).mock.calls.filter(([path]) => path === f.dmgPath),
    ).toHaveLength(1);
    expect(
      vi
        .mocked(fs.readFileSync)
        .mock.calls.filter(([path]) => path === join(f.destination, f.name)),
    ).toHaveLength(1);
    expect(fs.readdirSync(f.destination).sort()).toEqual(
      [f.name, "SHA256SUMS", "release-manifest.json", "release-notes.md"].sort(),
    );
    const manifest = verifyArtifactV2(f.destination, {
      tag: `v${f.version}`,
      source: f.source,
      repository: "owner/repo",
    });
    const sums = `${sha256(f.dmg)}  ${f.name}\n`;
    expect(manifest.assets).toEqual([
      { name: f.name, size: f.dmg.length, sha256: sha256(f.dmg) },
      { name: "SHA256SUMS", size: Buffer.byteLength(sums), sha256: sha256(sums) },
    ]);
    expect(fs.readFileSync(join(f.destination, "SHA256SUMS"), "utf8")).toBe(sums);
    expect(manifest.notesSha256).toBe(
      sha256(fs.readFileSync(join(f.destination, "release-notes.md"))),
    );
    for (const path of CONFIGURATION_FILES)
      expect(manifest.configuration[path]).toBe(sha256(fs.readFileSync(join(f.directory, path))));

    // Existing schema 1 artifacts remain independently readable; no schema 1 writer is needed.
    const {
      controller: _controller,
      workflow: _workflow,
      verificationAttempt: _verification,
      ...historical
    } = manifest;
    fs.writeFileSync(manifestPath, JSON.stringify({ ...historical, schema: 1 }));
    expect(verifyArtifact(f.destination, { ...manifest }).schema).toBe(1);
    fs.writeFileSync(join(f.destination, f.name), "tampered");
    expect(() => verifyArtifact(f.destination, { ...manifest })).toThrow("size/digest mismatch");
  });
});
