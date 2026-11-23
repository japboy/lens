import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { WebviewPort } from "./webview-port";

import type { AccessibilityPermissionState, SourceCapability } from "ui/contracts/resource-state";

export class AccessibilityPermissionController implements ReactiveController {
  state: AccessibilityPermissionState = { stage: "inactive" };

  private connected = false;
  private active = false;
  private generation = 0;
  private inspection = 0;
  private inspectionPending = false;
  private requestPending = false;
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

  async request(capability: SourceCapability): Promise<void> {
    if (!this.active || !this.connected || this.requestPending || this.state.stage !== "ready")
      return;
    const report = this.state.capabilities.find((report) => report.capability === capability);
    if (report?.status !== "permission_required" || report.request_action !== "request_permission")
      throw new Error("This capability has no supported permission request action.");
    const generation = this.generation;
    this.requestPending = true;
    this.inspection += 1;
    this.setState({ stage: "checking" });
    try {
      await this.port.requestSourceAccess(capability);
    } catch (error) {
      if (generation === this.generation && this.active && this.connected) {
        this.setState({ stage: "failed", message: String(error) });
      }
      throw error;
    } finally {
      // A pending native prompt survives view navigation; only its own completion
      // releases this single-flight guard.
      this.requestPending = false;
    }
    if (generation !== this.generation || !this.active || !this.connected) return;
    await this.refresh();
    if (generation !== this.generation || !this.active || !this.connected) return;
    if (this.followUpTimer !== undefined) window.clearTimeout(this.followUpTimer);
    this.followUpTimer = window.setTimeout(() => void this.refresh(), 1_200);
  }

  async refresh(): Promise<void> {
    if (
      !this.active ||
      !this.connected ||
      this.requestPending ||
      this.inspectionPending ||
      document.visibilityState !== "visible"
    )
      return;
    const generation = this.generation;
    const inspection = this.inspection;
    this.inspectionPending = true;
    try {
      const capabilities = await this.port.inspectSourceAccess();
      if (
        generation !== this.generation ||
        inspection !== this.inspection ||
        this.requestPending ||
        !this.active ||
        !this.connected
      )
        return;
      this.setState({ stage: "ready", capabilities });
    } catch (error) {
      if (
        generation !== this.generation ||
        inspection !== this.inspection ||
        this.requestPending ||
        !this.active ||
        !this.connected
      )
        return;
      this.setState({ stage: "failed", message: String(error) });
    } finally {
      this.inspectionPending = false;
    }
  }

  private start(): void {
    this.stop();
    this.generation += 1;
    this.setState({ stage: "checking" });
    void this.refresh();
    this.pollTimer = window.setInterval(() => void this.refresh(), 2_000);
  }

  private stop(): void {
    this.generation += 1;
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
