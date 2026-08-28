import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AgentKind, AppSnapshot } from "../types";

export type Unlisten = () => void;

export interface WebviewPort {
  subscribeToAppSnapshot(listener: (snapshot: AppSnapshot) => void): Promise<Unlisten>;
  getAppSnapshot(): Promise<AppSnapshot>;
  getAccessibilityPermission(): Promise<boolean>;
  requestAccessibilityPermission(): Promise<boolean>;
  setAgent(agent: AgentKind): Promise<void>;
  authenticateAgentSelection(methodId: string): Promise<void>;
  reauthenticateAgentSelection(): Promise<void>;
  signOutAgentSelection(): Promise<void>;
  setResponsePrompt(responsePrompt: string): Promise<void>;
  resetResponsePrompt(): Promise<void>;
  chooseDirectory(defaultPath?: string): Promise<string | undefined>;
  setWorkingDirectory(path: string): Promise<void>;
  addLensTarget(operationId: string): Promise<void>;
  removeLensTarget(operationId: string, targetId: string): Promise<void>;
  confirmLensTargets(operationId: string): Promise<void>;
  transformLens(operationId: string): Promise<void>;
  authenticateAgent(operationId: string, methodId: string): Promise<void>;
  cancelAgent(operationId: string, runId: string): Promise<void>;
  confirmAction(message: string, title: string): Promise<boolean>;
  openExternalUrl(url: string): Promise<void>;
  closeCurrentWindow(): Promise<void>;
}

export const tauriWebviewPort: WebviewPort = {
  async subscribeToAppSnapshot(listener) {
    return listen<AppSnapshot>("app-state-changed", ({ payload }) => listener(payload));
  },
  getAppSnapshot: () => invoke<AppSnapshot>("get_app_snapshot"),
  getAccessibilityPermission: () => invoke<boolean>("accessibility_permission"),
  requestAccessibilityPermission: () => invoke<boolean>("request_accessibility_permission"),
  async setAgent(agent) {
    await invoke("set_agent", { agent });
  },
  async authenticateAgentSelection(methodId) {
    await invoke("authenticate_agent_selection", { methodId });
  },
  async reauthenticateAgentSelection() {
    await invoke("reauthenticate_agent_selection");
  },
  async signOutAgentSelection() {
    await invoke("sign_out_agent_selection");
  },
  async setResponsePrompt(responsePrompt) {
    await invoke("set_response_prompt", { responsePrompt });
  },
  async resetResponsePrompt() {
    await invoke("reset_response_prompt");
  },
  async chooseDirectory(defaultPath) {
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath,
      title: "Choose Working Directory",
    });
    return typeof selected === "string" ? selected : undefined;
  },
  async setWorkingDirectory(path) {
    await invoke("set_working_directory", { path });
  },
  async addLensTarget(operationId) {
    await invoke("add_lens_target", { operationId });
  },
  async removeLensTarget(operationId, targetId) {
    await invoke("remove_lens_target", { operationId, targetId });
  },
  async confirmLensTargets(operationId) {
    await invoke("confirm_lens_targets", { operationId });
  },
  async transformLens(operationId) {
    await invoke("transform_lens", { operationId });
  },
  async authenticateAgent(operationId, methodId) {
    await invoke("authenticate_agent", { operationId, methodId });
  },
  async cancelAgent(operationId, runId) {
    await invoke("cancel_agent", { operationId, runId });
  },
  confirmAction: (message, title) => confirm(message, { title, kind: "warning" }),
  openExternalUrl: (url) => openUrl(url),
  async closeCurrentWindow() {
    await getCurrentWindow().close();
  },
};
