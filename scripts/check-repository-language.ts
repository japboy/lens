import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { extname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPOSITORY_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const BINARY_EXTENSIONS = new Set([".icns", ".ico", ".png"]);
const LETTER = /\p{Letter}/gu;
const LATIN_LETTER = /\p{Script=Latin}/u;
const UTF8_DECODER = new TextDecoder("utf-8", { fatal: true });

type ScanResult = {
  scanned: boolean;
  violations: string[];
};

function repositoryFiles(): string[] {
  const output = execFileSync(
    "git",
    ["-C", REPOSITORY_ROOT, "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    { encoding: "utf8" },
  );

  return output
    .split("\0")
    .filter(
      (relativePath) => Boolean(relativePath) && existsSync(resolve(REPOSITORY_ROOT, relativePath)),
    )
    .sort();
}

function nonLatinLetters(text: string): string[] {
  return [...text.matchAll(LETTER)]
    .map((match) => match[0])
    .filter((letter) => !LATIN_LETTER.test(letter));
}

function scanFile(relativePath: string): ScanResult {
  if (BINARY_EXTENSIONS.has(extname(relativePath).toLowerCase())) {
    return { scanned: false, violations: [] };
  }

  const content = readFileSync(resolve(REPOSITORY_ROOT, relativePath));
  let text: string;
  try {
    text = UTF8_DECODER.decode(content);
  } catch {
    throw new Error(
      `A non-exempt source artifact is neither UTF-8 text nor an allowed binary asset: ${relativePath}`,
    );
  }

  const violations = [];
  for (const [index, line] of text.split(/\r?\n/u).entries()) {
    const letters = [...new Set(nonLatinLetters(line))];
    if (letters.length === 0) continue;
    const excerpt = line.trim().slice(0, 160);
    violations.push(
      `${relativePath}:${index + 1}: non-Latin letter(s) ${letters
        .map((letter) => JSON.stringify(letter))
        .join(", ")} in ${JSON.stringify(excerpt)}`,
    );
  }
  return { scanned: true, violations };
}

const files = repositoryFiles();
const scanResults = files.map(scanFile);
const violations = scanResults.flatMap((result) => result.violations);

if (violations.length > 0) {
  process.stderr.write(["Repository language policy failed:", ...violations].join("\n") + "\n");
  process.exitCode = 1;
} else {
  const scanned = scanResults.filter((result) => result.scanned).length;
  process.stdout.write(
    `Repository language policy passed: ${scanned} UTF-8 text artifacts scanned.\n`,
  );
}
