#!/usr/bin/env node
// Run with Node 24: node apps/desktop/agent-icons/generate.ts [--check]
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const directory = fileURLToPath(new URL(".", import.meta.url));
const desktop = fileURLToPath(new URL("../", import.meta.url));
const catalog = JSON.parse(readFileSync(join(directory, "catalog.json"), "utf8")) as {
  version: number;
  assets: Record<string, string>;
  rules: { containsAny: string[]; icon: string }[];
  fallback: string;
};
if (catalog.version !== 1 || !catalog.assets[catalog.fallback]) {
  throw new Error("Invalid Agent icon catalog version or fallback");
}
for (const rule of catalog.rules) {
  if (
    !catalog.assets[rule.icon] ||
    !rule.containsAny.length ||
    rule.containsAny.some((part) => !/^[a-z]+$/.test(part))
  ) {
    throw new Error("Agent icon rules require known assets and lowercase ASCII substrings");
  }
}
const check = process.argv.includes("--check");
const staging = mkdtempSync(join(tmpdir(), "lens-agent-icons-"));
function output(name: string, content: Buffer) {
  const path = join(directory, name);
  if (check) {
    if (!readFileSync(path).equals(content)) throw new Error(`Stale Agent icon: ${name}`);
  } else {
    writeFileSync(path, content);
  }
}
try {
  for (const [name, source] of Object.entries(catalog.assets)) {
    const svg = readFileSync(
      join(desktop, "node_modules/@fortawesome/fontawesome-free/svgs", source),
      "utf8",
    );
    // Native PNGs need a square canvas; web masks center the original package SVG with CSS.
    const normalized = svg.replace(/viewBox="0 0 (\d+) (\d+)"/, (_, w: string, h: string) => {
      const width = Number(w),
        height = Number(h),
        size = Math.max(width, height);
      return `viewBox="${(width - size) / 2} ${(height - size) / 2} ${size} ${size}"`;
    });
    const input = join(staging, `${name}.svg`);
    writeFileSync(input, normalized.replaceAll('fill="currentColor"', 'fill="#000000"'));
    execFileSync(
      process.execPath,
      [
        join(desktop, "node_modules/@tauri-apps/cli/tauri.js"),
        "icon",
        input,
        "--png",
        "32",
        "--output",
        staging,
      ],
      { cwd: desktop, stdio: "pipe" },
    );
    output(`${name}.png`, readFileSync(join(staging, "32x32.png")));
  }
} finally {
  rmSync(staging, { recursive: true, force: true });
}
