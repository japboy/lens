import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  normalizedIcns,
  parseContract,
  renderAndroidLayer,
  renderAppearance,
  renderMacosIcon,
  renderIosIcon,
  renderRichMark,
  renderTauriManifest,
  validateCanonicalSource,
} from "../mise-tasks/generate/icons.ts";

const source = readFileSync(
  new URL("../apps/desktop/src-tauri/icons/icon.svg", import.meta.url),
  "utf8",
);
const rawContract = JSON.parse(
  readFileSync(
    new URL("../apps/desktop/src-tauri/icons/icon-family.json", import.meta.url),
    "utf8",
  ),
) as Record<string, unknown>;
const contract = parseContract(rawContract);
const pathData = (svg: string) => [...svg.matchAll(/<path [^>]*\bd="([^"]+)"/gu)].map((m) => m[1]);

describe("icon source and routing contract", () => {
  it("accepts only the current finite contract", () => {
    expect(contract.version).toBe(3);
    expect(() => parseContract({ ...rawContract, version: 2 })).toThrow("version must be 3");
    expect(() => parseContract({ ...rawContract, unexpected: true })).toThrow("exactly");
  });

  it("rejects a duplicate or escaping source filename", () => {
    expect(() => parseContract({ ...rawContract, source: contract.outputs.macos })).toThrow(
      "unique",
    );
    expect(() => parseContract({ ...rawContract, source: "../icon.svg" })).toThrow(
      "plain filename",
    );
  });

  it("rejects non-template monochrome colors and non-Retina tray sizing", () => {
    expect(() =>
      parseContract({
        ...rawContract,
        appearances: {
          ...contract.appearances,
          monochrome: { color: "#ffffff" },
        },
      }),
    ).toThrow("#000000");
    expect(() => parseContract({ ...rawContract, tray: { ...contract.tray, pixels: 18 } })).toThrow(
      "twice",
    );
  });

  it("rejects an Android mark outside the documented safe zone", () => {
    expect(() =>
      parseContract({ ...rawContract, tauri: { ...contract.tauri, androidLogoDiameter: 67 } }),
    ).toThrow("48 through 66");
  });

  it("rejects effects, incomplete geometry, or an opaque canonical background", () => {
    expect(() => validateCanonicalSource(source)).not.toThrow();
    for (const invalid of [
      source.replace("</svg>", '<rect width="20" height="20" /></svg>'),
      source.replace('fill="currentColor"', 'fill="url(#gradient)"'),
      source.replace('id="lens-face-crescent"', 'id="lens-face-major"'),
      source.replace(' fill-rule="evenodd"', ""),
    ])
      expect(() => validateCanonicalSource(invalid)).toThrow(/canonical SVG|canonical frame/u);
  });

  it("keeps every authored path unchanged in material and appearance projections", () => {
    for (const svg of [
      renderRichMark(source),
      ...Object.entries(contract.appearances).map(([name, appearance]) =>
        renderAppearance(source, name as keyof typeof contract.appearances, appearance.color),
      ),
    ]) {
      expect(pathData(svg)).toEqual(pathData(source));
    }
  });

  it("keeps materials grayscale and highlights clipped inside the mark", () => {
    const rich = renderRichMark(source);
    for (const match of rich.matchAll(/stop-color="#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})"/gu)) {
      expect(match[1]).toBe(match[2]);
      expect(match[1]).toBe(match[3]);
    }
    expect(rich).toContain('clip-path="url(#mark-clip)"');
    expect(rich).not.toMatch(/<(?:filter|rect|image)\b/u);
    expect(renderAppearance(source, "monochrome", "#000000")).not.toMatch(
      /(?:Gradient|stroke|opacity|rect)/u,
    );
  });

  it("adds macOS composition without altering or duplicating canonical path data", () => {
    const macos = renderMacosIcon(renderRichMark(source), contract.application.macosScale);
    expect(pathData(macos)).toEqual(pathData(source));
    expect(macos).toContain('rx="4.45"');
    expect(macos).toContain("scale(0.84)");
  });

  it("gives iOS a full-bleed background before rasterization without a corner mask", () => {
    const ios = renderIosIcon(renderRichMark(source), contract.tauri.iosBackground);
    expect(ios).toContain('<rect width="20" height="20" fill="#ffffff" />');
    expect(ios).not.toContain("rx=");
    expect(pathData(ios)).toEqual(pathData(source));
  });

  it("gives rich and monochrome Android layers the same centered safe-zone transform", () => {
    const diameter = contract.tauri.androidLogoDiameter;
    const rich = renderAndroidLayer(renderRichMark(source), diameter);
    const mono = renderAndroidLayer(renderAppearance(source, "monochrome", "#000000"), diameter);
    const transform = /transform="([^"]+)"/u;
    expect(rich.match(transform)?.[1]).toBe(mono.match(transform)?.[1]);
    const scale = Number(rich.match(/scale\(([^)]+)\)/u)?.[1]);
    expect(108 * (16 / 20) * scale).toBeCloseTo(diameter);
    expect(diameter).toBeLessThanOrEqual(66);
    expect(pathData(rich)).toEqual(pathData(mono));
    expect(mono).not.toMatch(/(?:Gradient|stroke|opacity|rect)/u);
  });

  it("routes each platform independently from the tray", () => {
    expect(JSON.parse(renderTauriManifest(contract))).toEqual({
      default: contract.outputs.richMark,
      bg_color: contract.tauri.iosBackground,
      android_bg: contract.outputs.androidBackground,
      android_fg: contract.outputs.androidForeground,
      android_fg_scale: 100,
      android_monochrome: contract.outputs.androidMonochrome,
    });
    expect(contract.tray.appearance).toBe("monochrome");
  });
});

describe("deterministic ICNS container", () => {
  const chunk = (name: string, value: number) => {
    const result = Buffer.alloc(9);
    result.write(name);
    result.writeUInt32BE(9, 4);
    result[8] = value;
    return result;
  };
  const container = (chunks: Buffer[]) => {
    const header = Buffer.alloc(8);
    header.write("icns");
    header.writeUInt32BE(8 + chunks.reduce((length, value) => length + value.length, 0), 4);
    return Buffer.concat([header, ...chunks]);
  };

  it("ignores chunk order but preserves every payload byte", () => {
    const a = chunk("ic07", 1);
    const b = chunk("ic08", 2);
    expect(normalizedIcns(container([a, b]), "test")).toEqual(
      normalizedIcns(container([b, a]), "test"),
    );
    expect(normalizedIcns(container([a, b]), "test")).not.toEqual(
      normalizedIcns(container([a, chunk("ic08", 3)]), "test"),
    );
  });

  it("rejects corrupt headers and chunk lengths", () => {
    expect(() => normalizedIcns(Buffer.from("bad"), "test")).toThrow("ICNS");
    const invalid = container([chunk("ic07", 1)]);
    invalid.writeUInt32BE(100, 12);
    expect(() => normalizedIcns(invalid, "test")).toThrow("chunk length");
  });
});
