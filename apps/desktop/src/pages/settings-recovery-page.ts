import { ReactiveElement } from "lit";
import { customElement, state } from "lit/decorators.js";
import { invoke } from "@tauri-apps/api/core";
import {
  LensSettingsRecoveryView,
  type SettingsRecoveryInfo,
} from "../components/lens-settings-recovery-view";
import { PageAttachment } from "../rendering/page-attachment";
@customElement("lens-settings-recovery-page")
export class SettingsRecoveryPage extends ReactiveElement {
  @state() private info: SettingsRecoveryInfo | undefined;
  @state() private busy = false;
  @state() private error = "";
  @state() private confirming = false;
  private readonly attachment = new PageAttachment(
    this,
    () => this.view,
    () => void this.load(),
    [],
  );
  protected createRenderRoot(): HTMLElement {
    return this;
  }
  initialize() {
    return this.attachment.initialize();
  }
  private get view() {
    const view = this.querySelector("lens-settings-recovery-view");
    if (!(view instanceof LensSettingsRecoveryView)) throw new Error("Missing recovery view");
    return view;
  }
  connectedCallback() {
    super.connectedCallback();
    this.addEventListener("settings-recovery-action", this.handleAction);
  }
  disconnectedCallback() {
    super.disconnectedCallback();
    this.removeEventListener("settings-recovery-action", this.handleAction);
  }
  protected update(changed: Map<PropertyKey, unknown>) {
    super.update(changed);
    if (this.attachment.stage !== "active") return;
    Object.assign(this.view, {
      info: this.info,
      busy: this.busy,
      error: this.error,
      confirming: this.confirming,
    });
  }
  private async load() {
    try {
      this.info = await invoke<SettingsRecoveryInfo>("get_settings_recovery");
    } catch (error) {
      this.error = String(error);
    }
  }
  private handleAction = (event: Event) => {
    void this.act((event as CustomEvent<string>).detail);
  };
  private async act(action: string) {
    if (this.busy) return;
    if (action === "confirm") {
      this.confirming = true;
      return;
    }
    if (action === "cancel") {
      this.confirming = false;
      return;
    }
    this.busy = true;
    this.error = "";
    try {
      if (action === "open") await invoke("open_recovery_settings_file");
      else if (action === "retry") await invoke("retry_settings_recovery");
      else if (
        action === "restore" &&
        this.confirming &&
        this.info?.can_restore_prompt_presets &&
        this.info.digest
      )
        await invoke("restore_recovery_prompt_presets", { expectedDigest: this.info.digest });
    } catch (error) {
      this.error = String(error);
      this.confirming = false;
      await this.load();
    } finally {
      this.busy = false;
    }
  }
}
