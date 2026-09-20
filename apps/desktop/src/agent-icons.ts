import catalogJson from "../agent-icons/catalog.json?raw";
import claude from "../agent-icons/claude.svg";
import openai from "../agent-icons/openai.svg";
import copilot from "../agent-icons/copilot.svg";
import robot from "../agent-icons/robot.svg";

const icons = { claude, openai, copilot, robot };
type Icon = keyof typeof icons;
const catalog = JSON.parse(catalogJson) as {
  assets: Record<string, string>;
  rules: { containsAny: string[]; icon: Icon }[];
  fallback: Icon;
};
const knownIcon = (icon: string) => Object.hasOwn(icons, icon);
if (
  !knownIcon(catalog.fallback) ||
  catalog.rules.some((rule) => !knownIcon(rule.icon)) ||
  Object.keys(catalog.assets).sort().join() !== Object.keys(icons).sort().join()
) {
  throw new Error("Bundled Agent icon catalog does not match the available assets");
}

/** First catalog rule wins; only ASCII letters are case-folded. */
export function agentIcon(name: string): string {
  const normalized = name.replace(/[A-Z]/g, (letter) => letter.toLowerCase());
  const rule = catalog.rules.find(({ containsAny }) =>
    containsAny.some((part) => normalized.includes(part)),
  );
  return icons[rule?.icon ?? catalog.fallback];
}
