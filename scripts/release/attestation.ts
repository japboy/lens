import { execFileSync } from "node:child_process";
import { lstatSync } from "node:fs";
import { resolve } from "node:path";

export interface AttestationIdentity {
  repository: string;
  controller: string;
  runId: string;
  signingAttempt: string;
}

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

/** Verify the bundle bytes and the certificate identity, never a predicate-supplied invocation. */
export function verifyBundleAttestation(path: string, identity: AttestationIdentity): void {
  if (
    !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(identity.repository) ||
    !/^[0-9a-f]{40}$/.test(identity.controller) ||
    !/^[1-9][0-9]*$/.test(identity.runId) ||
    !/^[1-9][0-9]*$/.test(identity.signingAttempt)
  ) {
    throw new Error("Invalid attestation identity");
  }
  const bundle = resolve(path);
  if (!lstatSync(bundle).isFile()) throw new Error("Attested bundle must be a regular file");
  const output = execFileSync(
    "gh",
    [
      "attestation",
      "verify",
      bundle,
      "--repo",
      identity.repository,
      "--signer-workflow",
      `${identity.repository}/.github/workflows/release.yml`,
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
    { encoding: "utf8", timeout: 120_000, maxBuffer: 16 * 1024 * 1024 },
  );
  const results: unknown = JSON.parse(output);
  const invocation = `https://github.com/${identity.repository}/actions/runs/${identity.runId}/attempts/${identity.signingAttempt}`;
  // gh v2.96.0 returns successful verification results with Sigstore certificate.Summary:
  // https://github.com/sigstore/sigstore-go/blob/v1.2.1/pkg/fulcio/certificate/extensions.go
  // Fulcio derives runInvocationURI from the GitHub OIDC run_id and run_attempt claims.
  // The statement predicate is workflow-controlled and is deliberately not consulted.
  if (
    !Array.isArray(results) ||
    !results.some((result: unknown) => {
      const verification = record(record(result).verificationResult);
      const certificate = record(record(verification.signature).certificate);
      return certificate.runInvocationURI === invocation;
    })
  ) {
    throw new Error("No verified attestation matches the release verification run and attempt");
  }
}
