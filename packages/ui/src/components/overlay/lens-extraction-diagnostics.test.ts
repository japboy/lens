// @vitest-environment jsdom
import { beforeAll, afterEach, expect, it } from "vitest";
import type { LensExtractionDiagnostics } from "./lens-extraction-diagnostics";
import type { LensContext } from "../../contracts/lens";

beforeAll(async () => {
  await import("./lens-extraction-diagnostics");
});
afterEach(() => document.body.replaceChildren());

it("separates capture omissions from Agent image omissions and keeps pending coverage honest", async () => {
  const element = document.createElement(
    "lens-extraction-diagnostics",
  ) as LensExtractionDiagnostics;
  const context: LensContext = {
    schema_version: 1,
    context_id: "context",
    revision: 1,
    sources: [],
    media: [],
    media_omissions: [
      { target_id: "target", reason: "byte_budget", omitted_count: 2, detail: "capture budget" },
    ],
    quality: "partial",
    diagnostics: [],
  };
  element.context = context;
  element.inputStatus = "pending";
  document.body.append(element);
  await element.updateComplete;
  const captured = element.querySelector('[aria-labelledby="media-metrics-heading"]')!;
  const delivered = element.querySelector('[aria-labelledby="agent-delivery-heading"]')!;
  expect(captured.textContent?.replace(/\s+/g, " ")).toContain("Images omitted during capture 2");
  expect(delivered.textContent).toContain("coverage is pending");
  expect(delivered.textContent).not.toContain("2 images omitted for Agent");

  element.delivery = {
    mode: "text_only_partial",
    sources: [
      {
        source_id: "source-0",
        mode: "text_only_partial",
        omitted_media: [{ id: "media-0", reason: "image_not_supported" }],
      },
    ],
  };
  element.inputStatus = "partial";
  element.projectionHasLoss = true;
  element.inputDetailsLoading = true;
  await element.updateComplete;
  expect(captured.textContent?.replace(/\s+/g, " ")).toContain("Images omitted during capture 2");
  expect(delivered.textContent?.replace(/\s+/g, " ")).toContain("Images omitted for Agent 1");
  expect(delivered.textContent?.replace(/\s+/g, " ")).toContain(
    "Source 1: Text only; 1 image omitted for Agent",
  );
  expect(delivered.textContent).toContain("Image input not supported for this submission");
  const pendingProjection = element.querySelector('[aria-labelledby="projection-loss-heading"]')!;
  expect(pendingProjection.getAttribute("aria-busy")).toBe("true");
  expect(pendingProjection.textContent).not.toContain("Details loading");
  expect(pendingProjection.textContent).not.toContain("Details unavailable");

  element.input = {
    schema_version: 3,
    context_id: "context",
    context_revision: 1,
    sources: [
      {
        source_id: "source-0",
        target_id: "target",
        source_revision: 1,
        source: {
          application: "Fixture",
          window_title: "Window",
          bundle_id: "fixture",
          window_id: 1,
        },
        quality: "full",
        omissions: [
          { reason: "application_chrome", omitted_node_count: 0 },
          { reason: "token_budget", omitted_node_count: 2 },
          { reason: "resource_budget", omitted_node_count: 0 },
        ],
      },
    ],
    media: [],
    media_omissions: [],
    quality: "full",
  };
  element.inputDetailsLoading = false;
  await element.updateComplete;
  const projection = element.querySelector('[aria-labelledby="projection-loss-heading"]')!;
  expect(projection.textContent).toContain("2 omission records");
  expect(projection.textContent).toContain("Text budget: 1");
  expect(projection.textContent).toContain("Resource budget: 1");
  expect(projection.textContent).not.toContain("Application chrome");
  expect(projection.getAttribute("aria-busy")).toBe("false");
});

it("keeps pending extraction regions empty without claiming diagnostics are absent", async () => {
  const element = document.createElement(
    "lens-extraction-diagnostics",
  ) as LensExtractionDiagnostics;
  element.inputDetailsLoading = true;
  element.projectionHasLoss = false;
  document.body.append(element);
  await element.updateComplete;
  expect(element.querySelector(".extraction-diagnostics")?.getAttribute("aria-busy")).toBe("true");
  expect(element.textContent).not.toContain("Loading");
  expect(element.textContent).not.toContain("No extraction diagnostics are available");
  expect(element.textContent).toContain("No text, resource, or document content was omitted");

  element.inputDetailsLoading = false;
  await element.updateComplete;
  expect(element.querySelector(".extraction-diagnostics")?.getAttribute("aria-busy")).toBe("false");
  expect(element.textContent).toContain("No extraction diagnostics are available");
});
