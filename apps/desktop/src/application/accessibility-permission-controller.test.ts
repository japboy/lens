// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ReactiveControllerHost } from "lit";
import { AccessibilityPermissionController } from "./accessibility-permission-controller";
import {
  parseAccessibilityAccess,
  type AccessibilityAccess,
  type WebviewPort,
} from "./webview-port";

const required: AccessibilityAccess = {
  schema_version: 1,
  status: "permission_required",
  action: "request_permission",
};
const ready: AccessibilityAccess = { schema_version: 1, status: "ready" };
const controllers: AccessibilityPermissionController[] = [];
function setup(access: AccessibilityAccess) {
  const request = vi.fn<WebviewPort["requestAccessibilityPermission"]>(async () => ready);
  const get = vi.fn<WebviewPort["getAccessibilityPermission"]>(async () => access);
  const host = {
    addController: vi.fn<ReactiveControllerHost["addController"]>(),
    requestUpdate: vi.fn<ReactiveControllerHost["requestUpdate"]>(),
  } as unknown as ReactiveControllerHost;
  const controller = new AccessibilityPermissionController(host, {
    getAccessibilityPermission: get,
    requestAccessibilityPermission: request,
  } as unknown as WebviewPort);
  controllers.push(controller);
  controller.hostConnected();
  controller.setActive(true);
  return { controller, request, get };
}
afterEach(() => {
  controllers.splice(0).forEach((controller) => controller.hostDisconnected());
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("Accessibility access contract", () => {
  it("recovers inspection rejection on the next poll without requesting permission", async () => {
    vi.useFakeTimers();
    const { controller, get, request } = setup(required);
    await Promise.resolve();
    get.mockRejectedValueOnce(new Error("Transient IPC error"));
    await controller.refresh();
    expect(controller.state).toEqual({
      stage: "failed",
      message: "Error: Transient IPC error",
      origin: "inspection",
    });
    get.mockResolvedValue(ready);
    await vi.advanceTimersByTimeAsync(2_000);
    expect(controller.state.stage).toBe("allowed");
    expect(get).toHaveBeenCalledTimes(3);
    expect(request).not.toHaveBeenCalled();
  });
  it.each([
    ready,
    { schema_version: 1, status: "access_restricted" },
    { schema_version: 1, status: "unsupported" },
    { schema_version: 1, status: "failed", message: "Native failure" },
  ] as const)("does not poll terminal native access %j", async (access) => {
    vi.useFakeTimers();
    const { get } = setup(access);
    await Promise.resolve();
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(8_000);
    expect(get).toHaveBeenCalledTimes(1);
  });
  it("recovers hidden activation when the document becomes visible", async () => {
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    const { controller, get } = setup(ready);
    expect(get).not.toHaveBeenCalled();
    visibility.mockReturnValue("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    await Promise.resolve();
    expect(controller.state.stage).toBe("allowed");
  });
  it("does not supersede a slow poll and stops inspecting after ready", async () => {
    vi.useFakeTimers();
    const { controller, get } = setup(required);
    await Promise.resolve();
    let resolve!: (value: AccessibilityAccess) => void;
    get.mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    await vi.advanceTimersByTimeAsync(8_000);
    expect(get).toHaveBeenCalledTimes(2);
    resolve(ready);
    await Promise.resolve();
    expect(controller.state.stage).toBe("allowed");
    await vi.advanceTimersByTimeAsync(8_000);
    expect(get).toHaveBeenCalledTimes(2);
  });
  it("rejects an old inspection after a newer request fails", async () => {
    const { controller, get, request } = setup(required);
    await Promise.resolve();
    let resolve!: (value: AccessibilityAccess) => void;
    get.mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const inspection = controller.refresh();
    request.mockRejectedValue(new Error("Denied"));
    await controller.request();
    resolve(ready);
    await inspection;
    expect(controller.state).toEqual({
      stage: "failed",
      message: "Error: Denied",
      origin: "request",
    });
  });
  it("cleans up a pending follow-up and visibility listener on disconnect", async () => {
    vi.useFakeTimers();
    const { controller, get, request } = setup(required);
    await Promise.resolve();
    request.mockResolvedValue(required);
    await controller.request();
    controller.hostDisconnected();
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(8_000);
    expect(get).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });
  it.each([
    true,
    null,
    {},
    { schema_version: 2, status: "ready" },
    { schema_version: 1, status: "unknown" },
    { schema_version: 1, status: "permission_required" },
    { schema_version: 1, status: "permission_required", action: "elevate" },
    { schema_version: 1, status: "failed" },
    { schema_version: 1, status: "ready", action: "request_permission" },
  ])("rejects unknown wire state %j", (value) => {
    expect(() => parseAccessibilityAccess(value)).toThrow(
      "Unsupported Accessibility access response",
    );
  });
  it.each([
    [ready, "allowed"],
    [required, "required"],
    [{ schema_version: 1, status: "access_restricted" }, "restricted"],
    [{ schema_version: 1, status: "unsupported" }, "unsupported"],
    [{ schema_version: 1, status: "failed", message: "Unavailable" }, "failed"],
  ] as const)("maps %j and admits only declared permission action", async (access, stage) => {
    expect(parseAccessibilityAccess(access)).toEqual(access);
    const { controller, request } = setup(access);
    await Promise.resolve();
    expect(controller.state.stage).toBe(stage);
    await controller.request();
    expect(request).toHaveBeenCalledTimes(stage === "required" ? 1 : 0);
  });
  it("rejects request completion after deactivation", async () => {
    const { controller, request } = setup(required);
    await Promise.resolve();
    let resolve!: (value: AccessibilityAccess) => void;
    request.mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const pending = controller.request();
    controller.setActive(false);
    resolve(ready);
    await pending;
    expect(controller.state).toEqual({ stage: "inactive" });
  });
  it("contains request rejection and removes the permission action", async () => {
    const { controller, request } = setup(required);
    await Promise.resolve();
    request.mockRejectedValue(new Error("Unavailable"));
    await controller.request();
    expect(controller.state.stage).toBe("failed");
    await controller.request();
    expect(request).toHaveBeenCalledTimes(1);
  });
});
