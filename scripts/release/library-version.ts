import assert from "node:assert/strict";

// Renovate must update both occurrences; conformance must execute that exact pin.
export function assertReleasePleaseVersion(pin: unknown, schema: unknown, installed: string): void {
  assert.equal(typeof pin, "string", "Release Please dependency must be an exact version");
  assert.match(
    pin as string,
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u,
    "Release Please dependency must be an exact stable version",
  );
  assert.equal(installed, pin, "Installed Release Please must match the dependency pin");
  assert.equal(
    schema,
    `https://raw.githubusercontent.com/googleapis/release-please/v${installed}/schemas/config.json`,
    "Release Please schema must match the dependency pin",
  );
}
