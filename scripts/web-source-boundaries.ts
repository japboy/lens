import { MEMBERS } from "./workspace-policy.ts";
import { isTypescriptTestSupport } from "./typescript-test-paths.ts";
import { builtinModules } from "node:module";
import { dirname, resolve, relative } from "node:path";

/** Package runtime sources cannot escape through undeclared relative imports or
 * acquire desktop/Tauri authority. Tests and package build configs are separate. */
export function webSourceViolations(
  path: string,
  source: string,
  owner: string,
  runtime: "browser" | "node" = "browser",
): string[] {
  const errors: string[] = [];
  const admit = (specifier: string): void => {
    if (/^ui\/(?:tests\/|test-fixtures\/|test-setup$)/u.test(specifier))
      errors.push(`Test-only source reachable from runtime: ${specifier}`);
    if (
      specifier === "desktop" ||
      specifier.startsWith("desktop/") ||
      specifier.startsWith("@tauri-apps/") ||
      specifier.startsWith("virtual:lens") ||
      specifier.startsWith("apps/desktop")
    )
      errors.push(`Desktop/native import forbidden: ${specifier}`);
    if (
      owner.endsWith("adapter-mcp-apps-host") &&
      (specifier.startsWith("adapter-math-renderer") ||
        specifier.startsWith("adapter-mcp-apps-view"))
    )
      errors.push("Generic MCP Host cannot depend on a renderer/View");
    if (specifier.startsWith(".")) {
      const local = relative(resolve(owner), resolve(dirname(path), specifier)).replaceAll(
        "\\",
        "/",
      );
      if (isTypescriptTestSupport(`${owner}/${local}`))
        errors.push(`Test-only source reachable from runtime: ${specifier}`);
      if (local === ".." || local.startsWith("../"))
        errors.push(`Cross-package relative import: ${specifier}`);
      if (local.startsWith("src/node/") && !path.includes("/src/node/"))
        errors.push(`Node facet reachable from browser runtime: ${specifier}`);
    }
    if (
      (specifier.startsWith("node:") ||
        builtinModules.includes(specifier) ||
        specifier === "vite" ||
        MEMBERS.some(
          (member) =>
            member.ecosystem === "pnpm" &&
            member.implementation === "tooling" &&
            (specifier === member.name || specifier.startsWith(`${member.name}/`)),
        ) ||
        specifier === "adapter-math-renderer/node" ||
        specifier === "adapter-math-renderer/html-math-manifest") &&
      runtime === "browser" &&
      !path.includes("/src/node/")
    )
      errors.push(`Node/build dependency in browser runtime: ${specifier}`);
  };

  errors.push(...moduleImportViolations(source, admit));
  return errors;
}

function moduleImportViolations(
  source: string,
  admit: (specifier: string) => void,
  inspectTemplateBodies = true,
): string[] {
  const errors: string[] = [];
  // Conservative lexical admission, not a compiler parser. Type checking owns
  // syntax. Comment/string contents are never interpreted as executable imports.
  const tokens = [
    ...source.matchAll(
      /\/\/[^\n]*|\/\*[\s\S]*?\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|`(?:\\.|[^`\\])*`|[A-Za-z_$][\w$]*|[^\s]/gu,
    ),
  ]
    .map((match) => match[0])
    .filter((token) => !token.startsWith("//") && !token.startsWith("/*"));
  const literal = (token: string | undefined): void => {
    if (!token || !/^["'][^\\]*["']$/u.test(token)) {
      errors.push("Module import requires an explicit unescaped string literal");
      return;
    }
    admit(token.slice(1, -1));
  };
  for (const [index, token] of tokens.entries()) {
    if (inspectTemplateBodies && token.startsWith("`") && /\b(?:import|require)\s*\(/u.test(token))
      errors.push("Module import inside template interpolation is not admitted");
    if (
      !inspectTemplateBodies &&
      token.startsWith("`") &&
      /\$\{[^}]*\b(?:import|require)\s*\(/u.test(token)
    )
      errors.push("Module import inside template interpolation is not admitted");
    if (token === "from" && /^["']/u.test(tokens[index + 1] ?? "")) literal(tokens[index + 1]);
    if (token === "import" || token === "require") {
      if (tokens[index + 1] === "(") literal(tokens[index + 2]);
      else if (token === "import" && /^["']/u.test(tokens[index + 1] ?? ""))
        literal(tokens[index + 1]);
    }
  }
  return errors;
}

/** All source owners use package exports when crossing into a shared package.
 * Repository orchestration may still reference application-specific files. */
export function crossPackageRelativeImportViolations(
  path: string,
  source: string,
  packageDirectories: readonly string[],
): string[] {
  const errors: string[] = [];
  errors.push(
    ...moduleImportViolations(
      source,
      (specifier) => {
        if (!specifier.startsWith(".")) return;
        const destination = resolve(dirname(path), specifier);
        const target = packageDirectories.find((directory) => {
          const within = relative(resolve(directory), destination).replaceAll("\\", "/");
          return within !== ".." && !within.startsWith("../");
        });
        if (!target) return;
        const within = relative(resolve(target), resolve(path)).replaceAll("\\", "/");
        if (within === ".." || within.startsWith("../"))
          errors.push(
            `Cross-package relative import requires a public package export: ${specifier}`,
          );
      },
      false,
    ),
  );
  return errors;
}
