/** Preview only. Native save parsing is authoritative. Quotes follow POSIX shlex,
 * but unquoted shell operators/expansions are rejected because no shell is run. */
function parseWords(input: string): string[] {
  const words: string[] = [];
  let word = "";
  let started = false;
  let quote: "single" | "double" | undefined;
  if (/[\r\n]/.test(input)) throw new Error("Use a single-line command without line breaks.");
  if (input.includes("\0")) throw new Error("Commands cannot contain NUL characters.");
  for (let index = 0; index < input.length; index += 1) {
    const character = input[index]!;
    if (character === "\\" && quote !== "single") {
      const next = input[index + 1];
      if (next === undefined) throw new Error("Complete the escape after the final backslash.");
      if (quote === "double" && !'$`"\\\n'.includes(next)) {
        word += character;
      } else {
        index += 1;
        if (next !== "\n") {
          word += next;
          started = true;
        }
      }
    } else if (character === "'" && quote !== "double") {
      quote = quote === "single" ? undefined : "single";
      started = true;
    } else if (character === '"' && quote !== "single") {
      quote = quote === "double" ? undefined : "double";
      started = true;
    } else if (/[\t\n\r ]/.test(character) && !quote) {
      if (started) words.push(word);
      word = "";
      started = false;
    } else {
      if (!quote && "|;&<>$`()~#".includes(character))
        throw new Error(
          "Quote literal special characters. Shell operators and expansion are not supported.",
        );
      word += character;
      started = true;
    }
  }
  if (quote) throw new Error("Close the quoted command or argument.");
  if (started) words.push(word);
  return words;
}

export function parseExternalAgentCommand(input: string): { command: string; args: string[] } {
  const [command, ...args] = parseWords(input);
  if (!command) throw new Error("Enter an executable and any arguments.");
  return { command, args };
}

export function parseExternalAgentArguments(input: string): string[] {
  return parseWords(input);
}

export function formatExternalAgentArguments(args: string[]): string {
  return args.map(quoteWord).join(" ");
}

function quoteWord(word: string): string {
  return /^[\w./:@=+-]+$/.test(word) ? word : `'${word.replaceAll("'", "'\\''")}'`;
}

export function formatExternalAgentCommand(profile: { command: string; args: string[] }): string {
  return [profile.command, ...profile.args].map(quoteWord).join(" ");
}
