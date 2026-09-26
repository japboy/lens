import { expect, it } from "vitest";
import {
  formatExternalAgentCommand,
  parseExternalAgentCommand,
  parseExternalAgentArguments,
  formatExternalAgentArguments,
} from "./external-agent-command";
it.each([
  { command: "goose", args: ["acp"] },
  { command: "/path with spaces/agent", args: ["", " ' \" ", "$VALUE", "a\\b"] },
  { command: "C:\\Program Files\\agent.exe", args: ["acp"] },
])("round-trips structured literal command %j", (profile) => {
  expect(parseExternalAgentCommand(formatExternalAgentCommand(profile))).toEqual(profile);
});
it.each([[], [""], ["acp", "", "two words", "a'b", "C:\\Program Files\\agent"]])(
  "round-trips independent arguments %j",
  (...args) => {
    expect(parseExternalAgentArguments(formatExternalAgentArguments(args))).toEqual(args);
  },
);
it("previews POSIX quotes and backslash rules without expansion", () => {
  expect(parseExternalAgentCommand(`goose 'a b' "c\\qd" "\\$literal" '' a\\ b`)).toEqual({
    command: "goose",
    args: ["a b", "c\\qd", "$literal", "", "a b"],
  });
});
it.each([
  "",
  "'' acp",
  "goose 'unfinished",
  "goose \\",
  "goose | other",
  "goose $HOME",
  "~/goose acp",
  "goose #note",
  "goose a\rb",
  "goose a\nb",
])("rejects invalid or unquoted shell syntax %s", (line) =>
  expect(() => parseExternalAgentCommand(line)).toThrow(
    /executable|quote|escape|Shell|single-line/,
  ),
);
