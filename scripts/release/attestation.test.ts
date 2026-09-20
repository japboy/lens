import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { verifyBundleAttestation } from "./attestation.ts";

vi.mock("node:child_process", () => ({ execFileSync: vi.fn<typeof execFileSync>() }));
const execute = vi.mocked(execFileSync);
const identity = {
  repository: "japboy/lens",
  controller: "a".repeat(40),
  runId: "123",
  signingAttempt: "2",
};
const invocation = "https://github.com/japboy/lens/actions/runs/123/attempts/2";
let directory: string;
let bundle: string;

function verified(uri = invocation) {
  return {
    verificationResult: {
      signature: { certificate: { runInvocationURI: uri } },
      statement: { predicate: { runDetails: { metadata: { invocationId: invocation } } } },
    },
  };
}

beforeEach(() => {
  directory = mkdtempSync(join(tmpdir(), "lens-attestation-"));
  bundle = join(directory, "release-bundle.json");
  writeFileSync(bundle, "{}");
  execute.mockReset();
  execute.mockReturnValue(JSON.stringify([verified()]));
});
afterEach(() => rmSync(directory, { recursive: true, force: true }));

it("binds the verified bundle to the signer, controller, main ref and certificate run attempt", () => {
  verifyBundleAttestation(bundle, identity);
  expect(execute).toHaveBeenCalledWith(
    "gh",
    [
      "attestation",
      "verify",
      bundle,
      "--repo",
      identity.repository,
      "--signer-workflow",
      "japboy/lens/.github/workflows/release.yml",
      "--signer-digest",
      identity.controller,
      "--source-ref",
      "refs/heads/main",
      "--source-digest",
      identity.controller,
      "--predicate-type",
      "https://slsa.dev/provenance/v1",
      "--cert-oidc-issuer",
      "https://token.actions.githubusercontent.com",
      "--deny-self-hosted-runners",
      "--format",
      "json",
    ],
    expect.objectContaining({ encoding: "utf8", timeout: 120_000 }),
  );
});

it.each([
  "https://github.com/japboy/lens/actions/runs/123/attempts/1",
  "https://github.com/japboy/lens/actions/runs/999/attempts/2",
  "https://github.com/other/lens/actions/runs/123/attempts/2",
])("rejects a different certificate invocation despite matching predicate: %s", (uri) => {
  execute.mockReturnValue(JSON.stringify([verified(uri)]));
  expect(() => verifyBundleAttestation(bundle, identity)).toThrow("No verified attestation");
});

it.each([
  [],
  {},
  [{ attestation: verified() }],
  [{ verificationResult: { statement: verified() } }],
])("rejects missing verified certificate identity: %j", (output) => {
  execute.mockReturnValue(JSON.stringify(output));
  expect(() => verifyBundleAttestation(bundle, identity)).toThrow("No verified attestation");
});

it("requires gh cryptographic verification to succeed", () => {
  execute.mockImplementation(() => {
    throw new Error("verification failed");
  });
  expect(() => verifyBundleAttestation(bundle, identity)).toThrow("verification failed");
});

it("allows a matching verified certificate among multiple results", () => {
  execute.mockReturnValue(JSON.stringify([verified("different"), verified()]));
  verifyBundleAttestation(bundle, identity);
  expect(execute).toHaveBeenCalledOnce();
});

it.each([
  { repository: "--bad" },
  { controller: "main" },
  { runId: "0" },
  { signingAttempt: "1/2" },
])("rejects malformed identity before running gh: %j", (change) => {
  expect(() => verifyBundleAttestation(bundle, { ...identity, ...change })).toThrow(
    "Invalid attestation identity",
  );
  expect(execute).not.toHaveBeenCalled();
});

it("rejects a symlink bundle", () => {
  const link = join(directory, "link");
  symlinkSync(bundle, link);
  expect(() => verifyBundleAttestation(link, identity)).toThrow("regular file");
  expect(execute).not.toHaveBeenCalled();
});

it("binds a later promotion signature independently of earlier build and gate attempts", () => {
  const retry = { ...identity, runAttempt: "1", verificationAttempt: "1", signingAttempt: "2" };
  verifyBundleAttestation(bundle, retry);
  expect(execute).toHaveBeenCalledOnce();
  execute.mockReturnValue(
    JSON.stringify([verified("https://github.com/japboy/lens/actions/runs/123/attempts/1")]),
  );
  expect(() => verifyBundleAttestation(bundle, retry)).toThrow("No verified attestation");
});
