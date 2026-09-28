import { describe, expect, it } from "vitest";
import fixturesJson from "../agent-icons/fixtures.json?raw";
import { agentIcon } from "./agent-icons";
import claude from "@fortawesome/fontawesome-free/svgs/brands/claude.svg";
import openai from "@fortawesome/fontawesome-free/svgs/brands/openai.svg";
import copilot from "@fortawesome/fontawesome-free/svgs/brands/copilot.svg";
import xTwitter from "@fortawesome/fontawesome-free/svgs/brands/x-twitter.svg";
import robot from "@fortawesome/fontawesome-free/svgs/solid/robot.svg";
import google from "@fortawesome/fontawesome-free/svgs/brands/google.svg";
const assets: Record<string, string> = {
  claude,
  openai,
  copilot,
  robot,
  google,
  "x-twitter": xTwitter,
};

const fixtures = JSON.parse(fixturesJson) as { name: string; icon: string }[];

describe("shared Agent icon matching contract", () => {
  for (const { name, icon } of fixtures) {
    it(`${JSON.stringify(name)} resolves to ${icon}`, () => {
      expect(agentIcon(name)).toBe(assets[icon]);
    });
  }
});
