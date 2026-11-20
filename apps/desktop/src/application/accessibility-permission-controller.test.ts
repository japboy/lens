// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ReactiveControllerHost } from "lit";
import type { WebviewPort } from "./webview-port";
import { AccessibilityPermissionController } from "./accessibility-permission-controller";
import {
  SOURCE_CAPABILITIES,
  type AccessStatus,
  type CapabilityInspection,
} from "ui/contracts/resource-state";

const reports = (status: AccessStatus): CapabilityInspection[] =>
  SOURCE_CAPABILITIES.map((capability) => ({
    capability,
    status,
    request_action:
      capability === "structured_extraction" && status === "permission_required"
        ? "request_permission"
        : null,
    diagnostics: [],
  }));
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
async function setup() {
  const host = {
    addController: vi.fn<ReactiveControllerHost["addController"]>(),
    requestUpdate: vi.fn<ReactiveControllerHost["requestUpdate"]>(),
  } as unknown as ReactiveControllerHost;
  const inspect = vi
    .fn<() => Promise<CapabilityInspection[]>>()
    .mockResolvedValue(reports("permission_required"));
  const request = vi
    .fn<WebviewPort["requestSourceAccess"]>()
    .mockResolvedValue(reports("permission_required")[1]!);
  const controller = new AccessibilityPermissionController(host, {
    inspectSourceAccess: inspect,
    requestSourceAccess: request,
  } as unknown as WebviewPort);
  controller.hostConnected();
  controller.setActive(true);
  await Promise.resolve();
  return { controller, inspect, request };
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
});
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

it("inspection never requests; an explicit request reinspects current state and keeps polling", async () => {
  const { controller, inspect, request } = await setup();
  await controller.refresh();
  expect(request).not.toHaveBeenCalled();
  await controller.request("structured_extraction");
  expect(request).toHaveBeenCalledTimes(1);
  expect(controller.state).toEqual({
    stage: "ready",
    capabilities: reports("permission_required"),
  });
  inspect.mockResolvedValue(reports("available"));
  await vi.advanceTimersByTimeAsync(2000);
  expect(controller.state).toEqual({ stage: "ready", capabilities: reports("available") });
  inspect.mockResolvedValue(reports("permission_required"));
  await vi.advanceTimersByTimeAsync(2000);
  expect(controller.state).toEqual({
    stage: "ready",
    capabilities: reports("permission_required"),
  });
  controller.hostDisconnected();
});

it("recovers transient inspection errors without turning restricted or unsupported states into requests", async () => {
  const { controller, inspect, request } = await setup();
  inspect.mockRejectedValueOnce(new Error("temporary IPC failure"));
  await controller.refresh();
  expect(controller.state.stage).toBe("failed");
  for (const status of ["restricted", "unsupported", "failed"] as const) {
    inspect.mockResolvedValue(reports(status));
    await vi.advanceTimersByTimeAsync(2000);
    expect(controller.state).toEqual({ stage: "ready", capabilities: reports(status) });
    await expect(controller.request("structured_extraction")).rejects.toThrow(
      "no supported permission request",
    );
    expect(request).not.toHaveBeenCalled();
  }
  controller.hostDisconnected();
});

it("keeps one explicit native request pending through timer ticks and drops old inspections", async () => {
  const { controller, inspect, request } = await setup();
  await controller.refresh();
  const old = deferred<CapabilityInspection[]>();
  inspect.mockReturnValueOnce(old.promise);
  const earlier = controller.refresh();
  const native = deferred<CapabilityInspection>();
  request.mockReturnValueOnce(native.promise);
  const asking = controller.request("structured_extraction");
  old.resolve(reports("available"));
  await earlier;
  await vi.advanceTimersByTimeAsync(4000);
  expect(controller.state.stage).toBe("checking");
  await controller.request("structured_extraction");
  expect(request).toHaveBeenCalledTimes(1);
  native.resolve(reports("permission_required")[1]!);
  await asking;
  expect(controller.state).toEqual({
    stage: "ready",
    capabilities: reports("permission_required"),
  });
  controller.hostDisconnected();
});

it("allows a slow single-flight inspection to complete across polling ticks and ignores results after disconnect", async () => {
  const { controller, inspect } = await setup();
  const old = deferred<CapabilityInspection[]>();
  inspect.mockReturnValueOnce(old.promise);
  const earlier = controller.refresh();
  const calls = inspect.mock.calls.length;
  await vi.advanceTimersByTimeAsync(6000);
  expect(inspect).toHaveBeenCalledTimes(calls);
  old.resolve(reports("restricted"));
  await earlier;
  expect(controller.state).toEqual({ stage: "ready", capabilities: reports("restricted") });
  const stale = deferred<CapabilityInspection[]>();
  inspect.mockReturnValueOnce(stale.promise);
  const pending = controller.refresh();
  controller.hostDisconnected();
  stale.resolve(reports("available"));
  await pending;
  expect(controller.state).toEqual({ stage: "ready", capabilities: reports("restricted") });
});

it("keeps one native request in flight across deactivation and reopening", async () => {
  const { controller, inspect, request } = await setup();
  const native = deferred<CapabilityInspection>();
  request.mockReturnValueOnce(native.promise);
  const asking = controller.request("structured_extraction");
  const inspections = inspect.mock.calls.length;
  controller.setActive(false);
  controller.setActive(true);
  await vi.advanceTimersByTimeAsync(4000);
  expect(controller.state.stage).toBe("checking");
  expect(inspect).toHaveBeenCalledTimes(inspections);
  await controller.request("structured_extraction");
  expect(request).toHaveBeenCalledTimes(1);
  native.resolve(reports("permission_required")[1]!);
  await asking;
  await vi.advanceTimersByTimeAsync(2000);
  expect(controller.state.stage).toBe("ready");
  await controller.request("structured_extraction");
  expect(request).toHaveBeenCalledTimes(2);
  controller.hostDisconnected();
});
