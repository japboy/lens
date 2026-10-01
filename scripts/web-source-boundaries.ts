import { builtinModules } from "node:module";
import { dirname, resolve, relative } from "node:path";

/** Package runtime sources cannot escape through undeclared relative imports or
 * acquire desktop/Tauri authority. Tests and package build configs are separate. */
export function webSourceViolations(path: string, source: string, owner: string): string[] {
  const errors: string[] = [];
  const admit = (specifier: string): void => {
    if (
      specifier === "desktop" ||
      specifier.startsWith("desktop/") ||
      specifier.startsWith("@tauri-apps/") ||
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
      if (local === ".." || local.startsWith("../"))
        errors.push(`Cross-package relative import: ${specifier}`);
      if (local.startsWith("src/node/") && !path.includes("/src/node/"))
        errors.push(`Node facet reachable from browser runtime: ${specifier}`);
    }
    if (
      (specifier.startsWith("node:") ||
        builtinModules.includes(specifier) ||
        specifier === "vite" ||
        specifier === "adapter-math-renderer/node" ||
        specifier === "adapter-math-renderer/manifest") &&
      !path.includes("/src/node/")
    )
      errors.push(`Node/build dependency in browser runtime: ${specifier}`);
  };
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
    if (token.startsWith("`") && /\b(?:import|require)\s*\(/u.test(token))
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
