import { execFileSync, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  readlinkSync,
  readdirSync,
  rmSync,
  mkdirSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readVersion } from "./version.ts";

export function bundleContract(root: string) {
  const app = join(root, "apps/desktop/src-tauri");
  const base = JSON.parse(readFileSync(join(app, "tauri.conf.json"), "utf8"));
  const mac = JSON.parse(readFileSync(join(app, "tauri.macos.conf.json"), "utf8"));
  const { version } = readVersion(root);
  if (
    base.productName !== "Lens" ||
    base.identifier !== "com.github.japboy.lens" ||
    mac.bundle.macOS.minimumSystemVersion !== "15.2" ||
    mac.bundle.macOS.signingIdentity !== "-"
  )
    throw new Error("Unreviewed distribution identity, OS or signing contract");
  return {
    version,
    minimum: "15.2",
    product: "Lens",
    identifier: "com.github.japboy.lens",
    executable: "lens",
  };
}

export function verifyApp(bundle: string, contract: ReturnType<typeof bundleContract>): void {
  const info = join(bundle, "Contents/Info.plist");
  for (const [key, value] of Object.entries({
    CFBundleIdentifier: contract.identifier,
    CFBundleExecutable: contract.executable,
    CFBundleName: contract.product,
    CFBundleShortVersionString: contract.version,
    CFBundleVersion: contract.version,
    LSMinimumSystemVersion: contract.minimum,
  })) {
    const actual = execFileSync("plutil", ["-extract", key, "raw", "-o", "-", info], {
      encoding: "utf8",
    }).trim();
    if (actual !== value) throw new Error(`Bundle ${key} mismatch: ${actual}`);
  }
  if (
    execFileSync("lipo", ["-archs", join(bundle, "Contents/MacOS/lens")], {
      encoding: "utf8",
    }).trim() !== "arm64"
  )
    throw new Error("Unexpected bundle architecture");
  execFileSync("codesign", ["--verify", "--deep", "--strict", bundle], { stdio: "inherit" });
  const signature = spawnSync("codesign", ["-d", "--verbose=4", bundle], { encoding: "utf8" });
  if (
    signature.status !== 0 ||
    !/^Signature=adhoc$/mu.test(signature.stderr) ||
    /^Authority=/mu.test(signature.stderr)
  )
    throw new Error("Actual application signature is not ad-hoc");
}

export function verifyDmg(dmg: string, contract: ReturnType<typeof bundleContract>): void {
  execFileSync("hdiutil", ["verify", dmg], { stdio: "inherit" });
  const wrapper = spawnSync("codesign", ["-d", dmg], {
    encoding: "utf8",
    env: { ...process.env, LC_ALL: "C" },
  });
  if (wrapper.status === 0 || !wrapper.stderr.includes("not signed at all"))
    throw new Error("Expected an unsigned DMG wrapper");
  const temporary = realpathSync(mkdtempSync(join(tmpdir(), "lens-dmg-")));
  const mount = join(temporary, "mounted");
  mkdirSync(mount);
  let device: string | undefined;
  let attachedMount = false;
  try {
    const plist = execFileSync("hdiutil", [
      "attach",
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mount,
      "-plist",
      dmg,
    ]);
    attachedMount = true;
    const attached = JSON.parse(
      execFileSync("plutil", ["-convert", "json", "-o", "-", "-"], {
        input: plist,
        encoding: "utf8",
      }),
    );
    device = attached["system-entities"].find(
      (entry: Record<string, string>) => entry["mount-point"] === mount,
    )?.["dev-entry"];
    if (!device || !/^\/dev\/disk\d+(?:s\d+)*$/u.test(device))
      throw new Error("Missing mounted DMG device identity");
    const applications = readdirSync(mount).filter((name) => name.endsWith(".app"));
    if (
      JSON.stringify(applications) !== '["Lens.app"]' ||
      readlinkSync(join(mount, "Applications")) !== "/Applications"
    )
      throw new Error("Unexpected DMG installation layout");
    const application = join(mount, "Lens.app");
    verifyApp(application, contract);
    const copied = join(temporary, "Lens.app");
    execFileSync("ditto", [application, copied]);
    verifyApp(copied, contract);
    if (!existsSync(join(copied, "Contents/MacOS/lens")))
      throw new Error("Copied executable missing");
  } finally {
    // Detaching only makes sense once something is attached, and its own failure must not
    // replace the error that brought us here or skip the directory cleanup below — a
    // leaked mount is what makes the next `hdiutil attach` fail as busy.
    if (attachedMount) {
      // Mountpoint is a fallback cleanup target if structured attach parsing fails.
      const detached = spawnSync("hdiutil", ["detach", device ?? mount], { stdio: "inherit" });
      if (detached.status !== 0)
        console.error(`Unable to detach the verification mount at ${device ?? mount}`);
    }
    rmSync(temporary, { recursive: true, force: true });
  }
}
