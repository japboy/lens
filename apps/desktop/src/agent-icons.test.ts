import { describe, expect, it } from "vitest";
import fixturesJson from "../agent-icons/fixtures.json?raw";
import { agentIcon } from "./agent-icons";
import claude from "../agent-icons/claude.svg";
import openai from "../agent-icons/openai.svg";
import copilot from "../agent-icons/copilot.svg";
import robot from "../agent-icons/robot.svg";
const assets: Record<string, string> = { claude, openai, copilot, robot };

const fixtures = JSON.parse(fixturesJson) as { name: string; icon: string }[];

describe("shared Agent icon matching contract", () => {
  for (const { name, icon } of fixtures) {
    it(`${JSON.stringify(name)} resolves to ${icon}`, () => {
      expect(agentIcon(name)).toBe(assets[icon]);
    });
  }
});
