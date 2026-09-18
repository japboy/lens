import { describe, expect, it, vi } from "vitest";
import type { Request } from "./github.ts";
import { policyReader } from "./generate.ts";

describe("release policy capability", () => {
  it("allows only the fixed branch and positive numeric ruleset GET endpoints", async () => {
    const request = vi.fn<Request>(async <T>() => ({ enforcement: "active" }) as T);
    const policy = policyReader(request as Request);
    await policy("/rules/branches/main");
    await policy("/rulesets/21411388");
    expect(request.mock.calls).toEqual([
      ["/rules/branches/main", "GET"],
      ["/rulesets/21411388", "GET"],
    ]);
  });
  it.each([
    "/releases",
    "/pulls",
    "/rules/branches/other",
    "/rulesets/0",
    "/rulesets/1?x=1",
    "/rulesets/../releases",
    "/rulesets/1/",
  ])("rejects %s without using the credential", (path) => {
    const request = vi.fn<Request>();
    expect(() => policyReader(request as Request)(path)).toThrow("Invalid release policy endpoint");
    expect(request).not.toHaveBeenCalled();
  });
});
