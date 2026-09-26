import * as childProcess from "node:child_process";
import * as fs from "node:fs";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { verifyDmg } from "./bundle.ts";

vi.mock("node:child_process", () => ({
  execFileSync: vi.fn<typeof childProcess.execFileSync>(),
  spawnSync: vi.fn<typeof childProcess.spawnSync>(),
}));
vi.mock("node:fs", async (original) => {
  const actual = await original<typeof fs>();
  return { ...actual, rmSync: vi.fn<typeof fs.rmSync>(actual.rmSync) };
});
const contract = {
  version: "0.1.0",
  minimum: "15.2",
  product: "Lens",
  identifier: "com.github.japboy.lens",
  executable: "lens",
};
const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  vi.resetAllMocks();
  const actual = await vi.importActual<typeof fs>("node:fs");
  for (const directory of temporaryDirectories.splice(0))
    actual.rmSync(directory, { recursive: true, force: true });
});
async function fixture(fault?: "attach" | "signal" | "parse" | "verify" | "copy") {
  const actual = await vi.importActual<typeof fs>("node:fs");
  vi.mocked(fs.rmSync).mockImplementation(actual.rmSync);
  const primary = Object.assign(
    new Error(`Injected ${fault} failure`),
    fault === "signal" ? { status: null, signal: "SIGTERM" } : {},
  );
  const detach = new Error("Injected detach failure");
  let mount = "";
  let detachFails = false;
  vi.mocked(childProcess.spawnSync).mockImplementation(
    (file, args) =>
      ({
        status: args?.includes("--verbose=4") ? 0 : 1,
        stderr: args?.includes("--verbose=4") ? "Signature=adhoc\n" : "not signed at all",
      }) as ReturnType<typeof childProcess.spawnSync>,
  );
  vi.mocked(childProcess.execFileSync).mockImplementation(((file: string, args: string[]) => {
    if (file === "hdiutil" && args[0] === "attach") {
      mount = args[args.indexOf("-mountpoint") + 1]!;
      temporaryDirectories.push(dirname(mount));
      if (fault === "attach" || fault === "signal") throw primary;
      fs.mkdirSync(join(mount, "Lens.app"));
      fs.symlinkSync("/Applications", join(mount, "Applications"));
      return Buffer.from("attach plist");
    }
    if (file === "hdiutil" && args[0] === "detach" && detachFails) throw detach;
    if (file === "plutil" && args[0] === "-convert") {
      if (fault === "parse") throw primary;
      return JSON.stringify({
        "system-entities": [{ "mount-point": mount, "dev-entry": "/dev/disk42s1" }],
      });
    }
    if (file === "plutil" && args[0] === "-extract") {
      if (fault === "verify") throw primary;
      return (
        {
          CFBundleIdentifier: contract.identifier,
          CFBundleExecutable: contract.executable,
          CFBundleName: contract.product,
          CFBundleShortVersionString: contract.version,
          CFBundleVersion: contract.version,
          LSMinimumSystemVersion: contract.minimum,
        } as Record<string, string>
      )[args[1]!]!;
    }
    if (file === "lipo") return "arm64";
    if (file === "ditto") {
      if (fault === "copy") throw primary;
      const executable = join(args[1]!, "Contents/MacOS/lens");
      fs.mkdirSync(dirname(executable), { recursive: true });
      fs.writeFileSync(executable, "fixture");
    }
    return Buffer.from("");
  }) as typeof childProcess.execFileSync);
  return {
    primary,
    detach,
    failDetach: () => {
      detachFails = true;
    },
    mount: () => mount,
    temporary: () => dirname(mount),
  };
}

describe("DMG verification resource cleanup", () => {
  it("detaches the known device and removes the copied app after successful verification", async () => {
    const f = await fixture();
    verifyDmg("fixture.dmg", contract);
    expect(childProcess.execFileSync).toHaveBeenCalledWith("hdiutil", ["detach", "/dev/disk42s1"], {
      stdio: "inherit",
    });
    expect(fs.existsSync(f.temporary())).toBe(false);
  });
  it.each(["attach", "signal", "parse", "verify", "copy"] as const)(
    "preserves the primary %s failure and removes temporary files",
    async (fault) => {
      const f = await fixture(fault);
      expect(() => verifyDmg("fixture.dmg", contract)).toThrow(f.primary);
      expect(childProcess.execFileSync).toHaveBeenCalledWith(
        "hdiutil",
        ["detach", ["attach", "signal", "parse"].includes(fault) ? f.mount() : "/dev/disk42s1"],
        { stdio: "inherit" },
      );
      expect(fs.existsSync(f.temporary())).toBe(false);
    },
  );
  it("removes temporary files and surfaces detach-only failure", async () => {
    const f = await fixture();
    f.failDetach();
    expect(() => verifyDmg("fixture.dmg", contract)).toThrow(f.detach);
    expect(fs.existsSync(f.temporary())).toBe(false);
  });
  it.each(["attach", "signal", "parse", "verify", "copy"] as const)(
    "retains %s as the cause when detach also fails",
    async (fault) => {
      const f = await fixture(fault);
      f.failDetach();
      let thrown: unknown;
      try {
        verifyDmg("fixture.dmg", contract);
      } catch (error) {
        thrown = error;
      }
      expect(thrown).toBeInstanceOf(AggregateError);
      expect((thrown as AggregateError).cause).toBe(f.primary);
      expect((thrown as AggregateError).errors).toEqual([f.primary, f.detach]);
      expect(fs.existsSync(f.temporary())).toBe(false);
    },
  );
  it("attempts removal after detach failure and reports removal failure too", async () => {
    const f = await fixture("verify");
    f.failDetach();
    const removal = new Error("Read-only mount still present");
    vi.mocked(fs.rmSync).mockImplementation(() => {
      throw removal;
    });
    let thrown: unknown;
    try {
      verifyDmg("fixture.dmg", contract);
    } catch (error) {
      thrown = error;
    }
    expect(fs.rmSync).toHaveBeenCalledWith(f.temporary(), { recursive: true, force: true });
    expect((thrown as AggregateError).cause).toBe(f.primary);
    expect((thrown as AggregateError).errors).toEqual([f.primary, f.detach, removal]);
  });
});
