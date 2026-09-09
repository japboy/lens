import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { AccessibilityAccess, WebviewPort } from "./webview-port";

export type AccessibilityPermissionState =
  | { stage: "inactive" }
  | { stage: "checking" }
  | { stage: "allowed" }
  | { stage: "required" }
  | { stage: "restricted" }
  | { stage: "unsupported" }
  | { stage: "failed"; message: string };

export class AccessibilityPermissionController implements ReactiveController {
  state: AccessibilityPermissionState = { stage: "inactive" };

  private connected = false;
  private active = false;
  private generation = 0;
  private inFlight: "idle" | "inspection" | "request" = "idle";
  private readonly onVisibilityChange = (): void => {
    void this.refresh();
  };
  private pollTimer: number | undefined;
  private followUpTimer: number | undefined;

  constructor(
    private readonly host: ReactiveControllerHost,
    private readonly port: WebviewPort,
  ) {
    host.addController(this);
  }

  hostConnected(): void {
    this.connected = true;
    if (this.active) this.start();
  }

  hostDisconnected(): void {
    this.connected = false;
    this.stop();
  }

  setActive(active: boolean): void {
    if (this.active === active) return;
    this.active = active;
    if (!this.connected) return;
    if (active) {
      this.start();
    } else {
      this.stop();
      this.setState({ stage: "inactive" });
    }
  }

  async request(): Promise<void> {
    if (!this.active || !this.connected || this.state.stage !== "required") return;
    const generation = ++this.generation;
    this.inFlight = "request";
    this.setState({ stage: "checking" });
    try {
      const access = await this.port.requestAccessibilityPermission();
      if (generation !== this.generation || !this.active || !this.connected) return;
      this.applyAccess(access);
      if (access.status === "permission_required") {
        if (this.followUpTimer !== undefined) window.clearTimeout(this.followUpTimer);
        this.followUpTimer = window.setTimeout(() => void this.refresh(), 1_200);
      }
    } catch (error) {
      if (generation !== this.generation || !this.active || !this.connected) return;
      this.setState({ stage: "failed", message: String(error) });
    } finally {
      if (generation === this.generation) this.inFlight = "idle";
    }
  }

  async refresh(): Promise<void> {
    if (!this.active || !this.connected || document.visibilityState !== "visible") return;
    if (this.state.stage !== "required" && this.state.stage !== "checking") return;
    if (this.inFlight !== "idle") return;
    const generation = ++this.generation;
    this.inFlight = "inspection";
    try {
      const access = await this.port.getAccessibilityPermission();
      if (generation !== this.generation || !this.active || !this.connected) return;
      this.applyAccess(access);
    } catch (error) {
      if (generation !== this.generation || !this.active || !this.connected) return;
      this.setState({ stage: "failed", message: String(error) });
    } finally {
      if (generation === this.generation) this.inFlight = "idle";
    }
  }

  private start(): void {
    this.stop();
    this.generation += 1;
    this.setState({ stage: "checking" });
    document.addEventListener("visibilitychange", this.onVisibilityChange);
    void this.refresh();
    this.pollTimer = window.setInterval(() => {
      if (this.state.stage === "required") void this.refresh();
    }, 2_000);
  }

  private applyAccess(access: AccessibilityAccess): void {
    switch (access.status) {
      case "ready":
        this.setState({ stage: "allowed" });
        break;
      case "permission_required":
        this.setState({ stage: "required" });
        break;
      case "access_restricted":
        this.setState({ stage: "restricted" });
        break;
      case "unsupported":
        this.setState({ stage: "unsupported" });
        break;
      case "failed":
        this.setState({ stage: "failed", message: access.message });
        break;
    }
  }

  private stop(): void {
    this.generation += 1;
    this.inFlight = "idle";
    document.removeEventListener("visibilitychange", this.onVisibilityChange);
    if (this.pollTimer !== undefined) window.clearInterval(this.pollTimer);
    if (this.followUpTimer !== undefined) window.clearTimeout(this.followUpTimer);
    this.pollTimer = undefined;
    this.followUpTimer = undefined;
  }

  private setState(state: AccessibilityPermissionState): void {
    this.state = state;
    this.host.requestUpdate();
  }
}
