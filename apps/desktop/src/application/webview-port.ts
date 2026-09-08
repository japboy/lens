import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { confirm, open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import type {
  AgentKind,
  AgentPromptTemplate,
  AppSnapshot,
  AgentDefaults,
  InteractionResponse,
} from "../types";

export type Unlisten = () => void;

export interface AboutInfo {
  name: string;
  version: string;
  copyright: string;
}

export interface AboutDocuments {
  license: string;
  notice: string;
}

export interface WebviewPort {
  getHtmlOutput(operationId: string, representationId: string, resourceId: string): Promise<string>;
  getAboutInfo(): Promise<AboutInfo>;
  getAboutDocuments(): Promise<AboutDocuments>;
  showAbout(): Promise<void>;
  previewAgentModel(selectionId: string, configId: string, value?: string): Promise<void>;
  setAgentDefaults(
    selectionId: string,
    defaults: AgentDefaults,
    confirmPrivilege: boolean,
  ): Promise<void>;
  setSessionOption(
    operationId: string,
    instanceId: string,
    configRevision: number,
    configId: string,
    value: string,
  ): Promise<void>;
  respondAgentInteraction(
    operationId: string,
    instanceId: string,
    interactionId: string,
    response: InteractionResponse,
  ): Promise<void>;
  subscribeToAppSnapshot(listener: (snapshot: AppSnapshot) => void): Promise<Unlisten>;
  getAppSnapshot(): Promise<AppSnapshot>;
  getAccessibilityPermission(): Promise<boolean>;
  requestAccessibilityPermission(): Promise<boolean>;
  setAgent(agent: AgentKind): Promise<void>;
  authenticateAgentSelection(methodId: string): Promise<void>;
  reauthenticateAgentSelection(): Promise<void>;
  signOutAgentSelection(): Promise<void>;
  setAgentPromptTemplate(agentPromptTemplate: AgentPromptTemplate): Promise<void>;
  resetAgentPromptTemplate(): Promise<void>;
  chooseDirectory(defaultPath?: string): Promise<string | undefined>;
  setWorkingDirectory(path: string): Promise<void>;
  addLensTarget(operationId: string): Promise<void>;
  removeLensTarget(operationId: string, targetId: string): Promise<void>;
  confirmLensTargets(operationId: string): Promise<void>;
  retryLensTransform(operationId: string): Promise<void>;
  authenticateAgent(operationId: string, methodId: string): Promise<void>;
  cancelAgent(operationId: string, runId: string): Promise<void>;
  pauseLens(operationId: string): Promise<void>;
  resumeLens(operationId: string): Promise<void>;
  stopLens(operationId: string): Promise<void>;
  confirmAction(message: string, title: string): Promise<boolean>;
  openExternalUrl(url: string): Promise<void>;
  closeCurrentWindow(): Promise<void>;
}

export const tauriWebviewPort: WebviewPort = {
  getHtmlOutput: (operationId, representationId, resourceId) =>
    invoke<string>("get_html_output", { operationId, representationId, resourceId }),
  getAboutInfo: () => invoke<AboutInfo>("get_about_info"),
  getAboutDocuments: () => invoke<AboutDocuments>("get_about_documents"),
  showAbout: () => invoke<void>("show_about"),
  async previewAgentModel(selectionId, configId, value) {
    await invoke("preview_agent_model", { selectionId, configId, value });
  },
  async setAgentDefaults(selectionId, defaults, confirmPrivilege) {
    await invoke("set_agent_defaults", { selectionId, defaults, confirmPrivilege });
  },
  async setSessionOption(operationId, instanceId, configRevision, configId, value) {
    await invoke("set_session_option", {
      operationId,
      instanceId,
      configRevision,
      configId,
      value,
    });
  },
  async respondAgentInteraction(operationId, instanceId, interactionId, response) {
    await invoke("respond_agent_interaction", { operationId, instanceId, interactionId, response });
  },
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
  async setAgentPromptTemplate(agentPromptTemplate) {
    await invoke("set_agent_prompt_template", { agentPromptTemplate });
  },
  async resetAgentPromptTemplate() {
    await invoke("reset_agent_prompt_template");
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
  async retryLensTransform(operationId) {
    await invoke("retry_lens_transform", { operationId });
  },
  async authenticateAgent(operationId, methodId) {
    await invoke("authenticate_agent", { operationId, methodId });
  },
  async cancelAgent(operationId, runId) {
    await invoke("cancel_agent", { operationId, runId });
  },
  async pauseLens(operationId) {
    await invoke("pause_lens", { operationId });
  },
  async resumeLens(operationId) {
    await invoke("resume_lens", { operationId });
  },
  async stopLens(operationId) {
    await invoke("stop_lens", { operationId });
  },
  confirmAction: (message, title) => confirm(message, { title, kind: "warning" }),
  openExternalUrl: (url) => openUrl(url),
  async closeCurrentWindow() {
    await getCurrentWindow().close();
  },
};
