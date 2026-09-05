#!/usr/bin/env node
//MISE description = "Generate app resources"
//MISE dir = "{{config_root}}"

import { execFileSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPOSITORY_ROOT = resolve(fileURLToPath(new URL("../../", import.meta.url)));
const DESKTOP_DIRECTORY = resolve(REPOSITORY_ROOT, "apps/desktop");
const ICON_DIRECTORY = resolve(DESKTOP_DIRECTORY, "src-tauri/icons");
const FAMILY_CONTRACT_PATH = resolve(ICON_DIRECTORY, "icon-family.json");
const APPEARANCE_NAMES = ["light", "dark", "monochrome"] as const;
const OBSOLETE_PROJECTION_FILES = [
  "icon-regular.svg",
  "icon-solid.svg",
  "icon-solid-light.svg",
  "icon-solid-dark.svg",
  "icon-solid-monochrome.svg",
  "tray-icon-template.svg",
] as const;
const COLOR_PATTERN = /^#[0-9a-f]{6}$/u;

type AppearanceName = (typeof APPEARANCE_NAMES)[number];
export type IconFamilyContract = {
  version: 3;
  source: string;
  appearances: Record<AppearanceName, { color: string }>;
  projections: Record<AppearanceName, string>;
  outputs: {
    richMark: string;
    macos: string;
    ios: string;
    androidForeground: string;
    androidMonochrome: string;
    androidBackground: string;
    tauriManifest: string;
    trayRaster: string;
  };
  application: { treatment: "smoked-glass"; macosScale: number };
  tauri: {
    iosBackground: string;
    androidBackground: string;
    androidLogoDiameter: number;
  };
  tray: { appearance: "monochrome"; pixels: number; points: number };
};

function fail(message: string): never {
  throw new Error(message);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requireRecord(value: unknown, path: string): Record<string, unknown> {
  if (!isRecord(value)) fail(`${path} must be an object`);
  return value;
}

function requireString(value: unknown, path: string): string {
  if (typeof value !== "string" || value.length === 0) fail(`${path} must be a string`);
  return value;
}

function requireLiteral<const T extends string>(value: unknown, path: string, expected: T): T {
  if (value !== expected) fail(`${path} must be ${expected}`);
  return expected;
}

function requireColor(value: unknown, path: string): string {
  const color = requireString(value, path);
  if (!COLOR_PATTERN.test(color)) {
    fail(`${path} must be a lowercase six-digit hexadecimal color`);
  }
  return color;
}

function requireBasename(value: unknown, path: string): string {
  const name = requireString(value, path);
  if (/[/\\]/u.test(name) || name === "." || name === "..") {
    fail(`${path} must be a plain filename`);
  }
  return name;
}

function requireIntegerInRange(
  value: unknown,
  path: string,
  minimum: number,
  maximum: number,
): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value < minimum || value > maximum) {
    fail(`${path} must be an integer from ${minimum} through ${maximum}`);
  }
  return value;
}

function requireExactKeys(
  value: Record<string, unknown>,
  keys: readonly string[],
  path: string,
): void {
  if (Object.keys(value).toSorted().join(",") !== [...keys].toSorted().join(",")) {
    fail(`${path} must contain exactly ${keys.join(", ")}`);
  }
}

export function parseContract(input: unknown): IconFamilyContract {
  const raw = requireRecord(input, "icon family contract");
  requireExactKeys(
    raw,
    ["version", "source", "appearances", "projections", "outputs", "application", "tauri", "tray"],
    "icon family contract",
  );
  if (raw.version !== 3) fail("icon family contract version must be 3");

  const rawAppearances = requireRecord(raw.appearances, "appearances");
  const rawProjections = requireRecord(raw.projections, "projections");
  requireExactKeys(rawAppearances, APPEARANCE_NAMES, "appearances");
  requireExactKeys(rawProjections, APPEARANCE_NAMES, "projections");
  const appearances = Object.fromEntries(
    APPEARANCE_NAMES.map((name) => {
      const appearance = requireRecord(rawAppearances[name], `appearances.${name}`);
      requireExactKeys(appearance, ["color"], `appearances.${name}`);
      return [name, { color: requireColor(appearance.color, `appearances.${name}.color`) }];
    }),
  ) as Record<AppearanceName, { color: string }>;
  const projections = Object.fromEntries(
    APPEARANCE_NAMES.map((name) => [
      name,
      requireBasename(rawProjections[name], `projections.${name}`),
    ]),
  ) as Record<AppearanceName, string>;

  const rawOutputs = requireRecord(raw.outputs, "outputs");
  const rawApplication = requireRecord(raw.application, "application");
  const rawTauri = requireRecord(raw.tauri, "tauri");
  const rawTray = requireRecord(raw.tray, "tray");
  requireExactKeys(
    rawOutputs,
    [
      "richMark",
      "macos",
      "ios",
      "androidForeground",
      "androidMonochrome",
      "androidBackground",
      "tauriManifest",
      "trayRaster",
    ],
    "outputs",
  );
  requireExactKeys(rawApplication, ["treatment", "macosScale"], "application");
  requireExactKeys(
    rawTauri,
    ["iosBackground", "androidBackground", "androidLogoDiameter"],
    "tauri",
  );
  requireExactKeys(rawTray, ["appearance", "pixels", "points"], "tray");
  const contract: IconFamilyContract = {
    version: 3,
    source: requireBasename(raw.source, "source"),
    appearances,
    projections,
    outputs: {
      richMark: requireBasename(rawOutputs.richMark, "outputs.richMark"),
      macos: requireBasename(rawOutputs.macos, "outputs.macos"),
      ios: requireBasename(rawOutputs.ios, "outputs.ios"),
      androidForeground: requireBasename(rawOutputs.androidForeground, "outputs.androidForeground"),
      androidMonochrome: requireBasename(rawOutputs.androidMonochrome, "outputs.androidMonochrome"),
      androidBackground: requireBasename(rawOutputs.androidBackground, "outputs.androidBackground"),
      tauriManifest: requireBasename(rawOutputs.tauriManifest, "outputs.tauriManifest"),
      trayRaster: requireBasename(rawOutputs.trayRaster, "outputs.trayRaster"),
    },
    application: {
      treatment: requireLiteral(rawApplication.treatment, "application.treatment", "smoked-glass"),
      macosScale: requireIntegerInRange(
        rawApplication.macosScale,
        "application.macosScale",
        50,
        100,
      ),
    },
    tauri: {
      iosBackground: requireColor(rawTauri.iosBackground, "tauri.iosBackground"),
      androidBackground: requireColor(rawTauri.androidBackground, "tauri.androidBackground"),
      androidLogoDiameter: requireIntegerInRange(
        rawTauri.androidLogoDiameter,
        "tauri.androidLogoDiameter",
        48,
        66,
      ),
    },
    tray: {
      appearance: requireLiteral(rawTray.appearance, "tray.appearance", "monochrome"),
      pixels: requireIntegerInRange(rawTray.pixels, "tray.pixels", 1, 1024),
      points: requireIntegerInRange(rawTray.points, "tray.points", 1, 512),
    },
  };
  if (contract.appearances.monochrome.color !== "#000000") {
    fail("appearances.monochrome.color must be #000000 for a template image");
  }
  if (contract.tray.pixels !== contract.tray.points * 2) {
    fail("tray.pixels must be exactly twice tray.points for an @2x template image");
  }
  const filenames = [
    contract.source,
    ...Object.values(contract.projections),
    ...Object.values(contract.outputs),
  ];
  if (new Set(filenames).size !== filenames.length)
    fail("icon source and output filenames must be unique");
  if (filenames.some((name) => OBSOLETE_PROJECTION_FILES.some((obsolete) => obsolete === name))) {
    fail("the contract must not reference a retired Regular/Solid source or projection");
  }
  return contract;
}

export function validateCanonicalSource(svg: string): void {
  // Admit only the authored, opaque three-path mark. Materials belong to derivatives.
  const structure =
    /^<svg xmlns="http:\/\/www\.w3\.org\/2000\/svg" viewBox="0 0 20 20" role="img" aria-labelledby="title desc">\s*<title id="title">[^<>]+<\/title>\s*<desc id="desc">[^<>]+<\/desc>\s*<g fill="currentColor">\s*(?:<path id="(?:frame|lens-face-major|lens-face-crescent)"(?: fill-rule="evenodd")? d="[MAHZQV0-9 .-]+" \/>\s*){3}<\/g>\s*<\/svg>\s*$/u;
  if (!structure.test(svg)) {
    fail(
      "canonical SVG must be a 20-unit currentColor mark with exactly three filled paths, without strokes, masks, effects, or a background",
    );
  }
  for (const id of ["frame", "lens-face-major", "lens-face-crescent"]) {
    if (svg.split(`id="${id}"`).length !== 2) fail(`canonical SVG must contain ${id} exactly once`);
  }
  if (!svg.includes('id="frame" fill-rule="evenodd"')) {
    fail("canonical frame must preserve its transparent inner counter");
  }
}

export function renderAppearance(
  canonicalSvg: string,
  name: AppearanceName,
  color: string,
): string {
  return canonicalSvg
    .replace("<svg", `<svg data-lens-appearance="${name}"`)
    .replace('<g fill="currentColor">', `<g fill="${color}">`);
}

function renderAndroidBackground(color: string): string {
  return [
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" role="img" aria-labelledby="title">',
    '  <title id="title">Lens Android adaptive-icon background</title>',
    `  <rect width="1024" height="1024" fill="${color}" />`,
    "</svg>",
    "",
  ].join("\n");
}

export function renderRichMark(canonicalSvg: string): string {
  validateCanonicalSource(canonicalSvg);
  const paths = [...canonicalSvg.matchAll(/<path id="([^"]+)"[^>]+\/>/gu)];
  const references = paths.map((path) => `<use href="#shape-${path[1]}" />`).join("\n    ");
  return [
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20" role="img" aria-labelledby="title desc">',
    '  <title id="title">Lens</title>',
    '  <desc id="desc">Smoked-glass material derived from the canonical Lens mark.</desc>',
    "  <defs>",
    ...paths.map((path) => `    ${path[0].replace('id="', 'id="shape-')}`),
    '    <linearGradient id="rim" x1="15%" y1="0%" x2="70%" y2="100%">',
    '      <stop offset="0" stop-color="#979797" />',
    '      <stop offset=".22" stop-color="#434343" />',
    '      <stop offset=".62" stop-color="#222222" />',
    '      <stop offset="1" stop-color="#727272" />',
    "    </linearGradient>",
    '    <linearGradient id="face" x1="10%" y1="0%" x2="75%" y2="100%">',
    '      <stop offset="0" stop-color="#939393" />',
    '      <stop offset="1" stop-color="#333333" />',
    "    </linearGradient>",
    '    <radialGradient id="sheen" cx="30%" cy="12%" r="85%">',
    '      <stop offset="0" stop-color="#ffffff" stop-opacity=".55" />',
    '      <stop offset=".6" stop-color="#ffffff" stop-opacity=".03" />',
    '      <stop offset="1" stop-color="#ffffff" stop-opacity="0" />',
    "    </radialGradient>",
    `    <clipPath id="mark-clip">${references}</clipPath>`,
    "  </defs>",
    '  <g data-lens-mark="smoked-glass">',
    ...paths.map(
      (path) =>
        `    <use href="#shape-${path[1]}" fill="url(#${path[1] === "frame" ? "rim" : "face"})" />`,
    ),
    '    <g clip-path="url(#mark-clip)" fill="none" stroke="url(#sheen)" stroke-width=".16">',
    `      ${references}`,
    "    </g>",
    "  </g>",
    "</svg>",
    "",
  ].join("\n");
}

export function renderMacosIcon(richSvg: string, scalePercent: number): string {
  const scale = scalePercent / 100;
  const inset = 10 * (1 - scale);
  const plate = [
    '<defs><linearGradient id="plate" x1="0%" y1="0%" x2="30%" y2="100%">',
    '<stop offset="0" stop-color="#f8f8f8" /><stop offset="1" stop-color="#dddddd" />',
    "</linearGradient></defs>",
    '<rect width="20" height="20" rx="4.45" fill="url(#plate)" />',
  ].join("\n");
  // The legacy ICNS owns its inset/rounded plate; neither is part of the symbol.
  return richSvg
    .replace(
      '<g data-lens-mark="smoked-glass">',
      `<g transform="translate(${inset} ${inset}) scale(${scale})">\n${plate}\n<g data-lens-mark="smoked-glass">`,
    )
    .replace("</svg>", "</g>\n</svg>");
}

export function renderAndroidLayer(svg: string, diameter: number): string {
  // Canonical diameter 16/20; fit inside the documented 66dp zone of a 108dp layer.
  const scale = diameter / (108 * (16 / 20));
  return svg
    .replace(
      /<g (?:data-lens-mark="smoked-glass"|fill="#[0-9a-f]{6}")>/u,
      (group) => `<g transform="translate(10 10) scale(${scale}) translate(-10 -10)">\n${group}`,
    )
    .replace("</svg>", "</g>\n</svg>");
}

export function renderIosIcon(richSvg: string, background: string): string {
  // Rasterize the background with the artwork. CLI-only alpha compositing can
  // round a few edge pixels down to 254/255, which is not an opaque app icon.
  return richSvg.replace(
    '<g data-lens-mark="smoked-glass">',
    `<rect width="20" height="20" fill="${background}" />\n<g data-lens-mark="smoked-glass">`,
  );
}

export function renderTauriManifest(contract: IconFamilyContract): string {
  return `${JSON.stringify(
    {
      default: contract.outputs.richMark,
      bg_color: contract.tauri.iosBackground,
      android_bg: contract.outputs.androidBackground,
      android_fg: contract.outputs.androidForeground,
      // The CLI applies this only to legacy icons, not adaptive foregrounds.
      android_fg_scale: 100,
      android_monochrome: contract.outputs.androidMonochrome,
    },
    null,
    2,
  )}\n`;
}

function compareOrWrite(path: string, expected: string, check: boolean): void {
  if (check) {
    if (!existsSync(path) || readFileSync(path, "utf8") !== expected) {
      fail(`${relative(REPOSITORY_ROOT, path)} is not generated from icon-family.json`);
    }
    return;
  }
  writeFileSync(path, expected, "utf8");
}

function filesRecursively(root: string, directory = root): string[] {
  return readdirSync(directory)
    .sort()
    .flatMap((name) => {
      const path = join(directory, name);
      return statSync(path).isDirectory() ? filesRecursively(root, path) : [relative(root, path)];
    });
}

function runTauriIcon(arguments_: string[]): void {
  const executable = process.platform === "win32" ? "pnpm.cmd" : "pnpm";
  execFileSync(executable, ["exec", "tauri", "icon", ...arguments_], {
    cwd: DESKTOP_DIRECTORY,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function generateTauriResources(
  manifestPath: string,
  contract: IconFamilyContract,
  outputPath: string,
): void {
  runTauriIcon([manifestPath, "--output", outputPath]);
  const macosStaging = resolve(outputPath, ".macos-staging");
  runTauriIcon([resolve(ICON_DIRECTORY, contract.outputs.macos), "--output", macosStaging]);
  // The CLI has one default source for all platforms. Override only the ICNS;
  // iOS retains its full-bleed opaque background and no pre-rendered corner mask.
  copyFileSync(resolve(macosStaging, "icon.icns"), resolve(outputPath, "icon.icns"));
  rmSync(macosStaging, { recursive: true, force: true });
  const iosStaging = resolve(outputPath, ".ios-staging");
  runTauriIcon([
    resolve(ICON_DIRECTORY, contract.outputs.ios),
    "--output",
    iosStaging,
    "--ios-color",
    contract.tauri.iosBackground,
  ]);
  for (const filename of filesRecursively(resolve(iosStaging, "ios"))) {
    copyFileSync(resolve(iosStaging, "ios", filename), resolve(outputPath, "ios", filename));
  }
  rmSync(iosStaging, { recursive: true, force: true });
}

function generateTrayResource(
  monochromePath: string,
  contract: IconFamilyContract,
  outputPath: string,
): void {
  const stagingPath = resolve(outputPath, ".tray-staging");
  runTauriIcon([monochromePath, "--png", String(contract.tray.pixels), "--output", stagingPath]);
  const generatedPath = resolve(stagingPath, `${contract.tray.pixels}x${contract.tray.pixels}.png`);
  if (!existsSync(generatedPath)) fail(`Tauri did not generate ${generatedPath}`);
  copyFileSync(generatedPath, resolve(outputPath, contract.outputs.trayRaster));
  rmSync(stagingPath, { recursive: true, force: true });
}

export function normalizedIcns(content: Buffer, path: string): Buffer {
  if (content.length < 8 || content.toString("ascii", 0, 4) !== "icns") {
    fail(`${path} is not an ICNS container`);
  }
  const declaredLength = content.readUInt32BE(4);
  if (declaredLength !== content.length) fail(`${path} has an invalid ICNS length`);

  const chunks: Buffer[] = [];
  let offset = 8;
  while (offset < content.length) {
    if (offset + 8 > content.length) fail(`${path} has a truncated ICNS chunk header`);
    const chunkLength = content.readUInt32BE(offset + 4);
    if (chunkLength < 8 || offset + chunkLength > content.length) {
      fail(`${path} has an invalid ICNS chunk length`);
    }
    chunks.push(content.subarray(offset, offset + chunkLength));
    offset += chunkLength;
  }

  // Tauri's ICNS encoder preserves every chunk but emits their order nondeterministically.
  // Sorting complete chunks compares the semantic container without weakening payload checks.
  const sortedChunks = chunks.toSorted(Buffer.compare);
  const header = Buffer.alloc(8);
  header.write("icns", 0, "ascii");
  header.writeUInt32BE(8 + sortedChunks.reduce((length, chunk) => length + chunk.length, 0), 4);
  return Buffer.concat([header, ...sortedChunks]);
}

function generatedResource(path: string, relativePath: string): Buffer {
  const content = readFileSync(path);
  return relativePath === "icon.icns" ? normalizedIcns(content, path) : content;
}

function compareGeneratedResources(actualRoot: string, expectedRoot: string): void {
  const generatedFiles = filesRecursively(actualRoot);
  const mismatches = generatedFiles.filter((relativePath) => {
    const expectedPath = resolve(expectedRoot, relativePath);
    return (
      !existsSync(expectedPath) ||
      !generatedResource(resolve(actualRoot, relativePath), relativePath).equals(
        generatedResource(expectedPath, relativePath),
      )
    );
  });
  if (mismatches.length > 0) {
    fail(`generated Tauri icon resources differ: ${mismatches.join(", ")}`);
  }

  const generatedSet = new Set(generatedFiles);
  const staleFiles = managedResourceFiles(expectedRoot).filter(
    (relativePath) => !generatedSet.has(relativePath),
  );
  if (staleFiles.length > 0) fail(`stale Tauri icon resources exist: ${staleFiles.join(", ")}`);
}

function managedResourceFiles(root: string): string[] {
  return filesRecursively(root).filter((relativePath) => {
    if (relativePath.startsWith("android/") || relativePath.startsWith("ios/")) return true;
    if (relativePath.includes("/")) return false;
    return /\.(?:icns|ico|png)$/u.test(relativePath);
  });
}

function synchronizeGeneratedResources(generatedRoot: string, destinationRoot: string): void {
  const generatedFiles = filesRecursively(generatedRoot);
  const generatedSet = new Set(generatedFiles);
  for (const relativePath of generatedFiles) {
    const sourcePath = resolve(generatedRoot, relativePath);
    const destinationPath = resolve(destinationRoot, relativePath);
    mkdirSync(dirname(destinationPath), { recursive: true });
    if (relativePath === "icon.icns") {
      writeFileSync(destinationPath, normalizedIcns(readFileSync(sourcePath), sourcePath));
    } else {
      copyFileSync(sourcePath, destinationPath);
    }
  }
  for (const relativePath of managedResourceFiles(destinationRoot)) {
    if (!generatedSet.has(relativePath)) rmSync(resolve(destinationRoot, relativePath));
  }
}

function enforceNoObsoleteProjections(check: boolean): void {
  const obsolete = OBSOLETE_PROJECTION_FILES.filter((name) =>
    existsSync(resolve(ICON_DIRECTORY, name)),
  );
  if (check && obsolete.length > 0) {
    fail(`obsolete icon projections exist: ${obsolete.join(", ")}`);
  }
  for (const name of obsolete) rmSync(resolve(ICON_DIRECTORY, name));
}

function main(): void {
  const arguments_ = process.argv.slice(2);
  const check = arguments_.length === 1 && arguments_[0] === "--check";
  if (!check && arguments_.length !== 0) fail("usage: mise run generate:icons [--check]");

  const contract = parseContract(JSON.parse(readFileSync(FAMILY_CONTRACT_PATH, "utf8")) as unknown);
  const canonicalSvg = readFileSync(resolve(ICON_DIRECTORY, contract.source), "utf8");
  validateCanonicalSource(canonicalSvg);

  for (const name of APPEARANCE_NAMES) {
    compareOrWrite(
      resolve(ICON_DIRECTORY, contract.projections[name]),
      renderAppearance(canonicalSvg, name, contract.appearances[name].color),
      check,
    );
  }
  const richSvg = renderRichMark(canonicalSvg);
  const materialProjections = {
    [contract.outputs.richMark]: richSvg,
    [contract.outputs.macos]: renderMacosIcon(richSvg, contract.application.macosScale),
    [contract.outputs.ios]: renderIosIcon(richSvg, contract.tauri.iosBackground),
    [contract.outputs.androidForeground]: renderAndroidLayer(
      richSvg,
      contract.tauri.androidLogoDiameter,
    ),
    [contract.outputs.androidMonochrome]: renderAndroidLayer(
      renderAppearance(canonicalSvg, "monochrome", contract.appearances.monochrome.color),
      contract.tauri.androidLogoDiameter,
    ),
  };
  for (const [filename, svg] of Object.entries(materialProjections)) {
    compareOrWrite(resolve(ICON_DIRECTORY, filename), svg, check);
  }
  compareOrWrite(
    resolve(ICON_DIRECTORY, contract.outputs.androidBackground),
    renderAndroidBackground(contract.tauri.androidBackground),
    check,
  );
  const manifestPath = resolve(ICON_DIRECTORY, contract.outputs.tauriManifest);
  compareOrWrite(manifestPath, renderTauriManifest(contract), check);

  const temporaryDirectory = mkdtempSync(join(tmpdir(), "lens-icon-family-"));
  try {
    generateTauriResources(manifestPath, contract, temporaryDirectory);
    generateTrayResource(
      resolve(ICON_DIRECTORY, contract.projections[contract.tray.appearance]),
      contract,
      temporaryDirectory,
    );
    if (check) {
      compareGeneratedResources(temporaryDirectory, ICON_DIRECTORY);
    } else {
      synchronizeGeneratedResources(temporaryDirectory, ICON_DIRECTORY);
    }
  } finally {
    rmSync(temporaryDirectory, { recursive: true, force: true });
  }
  enforceNoObsoleteProjections(check);
  process.stdout.write(
    check
      ? "Icon family contract and generated resources are current.\n"
      : "Generated canonical projections, smoked-glass app resources, and template tray.\n",
  );
}

if (import.meta.main) main();
