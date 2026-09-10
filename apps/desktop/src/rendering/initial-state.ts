import type { AboutDocuments, AboutInfo } from "../application/webview-port";
import type {
  SettingsViewModel,
  OverlayViewModel,
  TargetSelectionViewModel,
} from "../application/view-models";
import type { SnapshotStatus } from "./snapshot-status";

export type Resource<T> =
  | { stage: "loading" }
  | { stage: "ready"; value: T }
  | { stage: "failed"; message: string };

export type DocumentKind = "license" | "notice";
export type AttachmentState = "prerendered" | "hydrating" | "active" | "failed";

export function initialAboutState(): {
  info: Resource<AboutInfo>;
  documents: Resource<AboutDocuments>;
  document: DocumentKind;
} {
  return { info: { stage: "loading" }, documents: { stage: "loading" }, document: "license" };
}

export function initialSettingsState() {
  return {
    model: undefined as SettingsViewModel | undefined,
    snapshotStatus: { stage: "loading" } as SnapshotStatus,
    destination: "connection" as const,
    windowEmphasis: "emphasized" as const,
    active: false,
  };
}
export function initialOverlayState() {
  return {
    model: undefined as OverlayViewModel | undefined,
    snapshotStatus: { stage: "loading" } as SnapshotStatus,
    activeTab: "interpretation" as const,
    notificationVisibility: "closed" as const,
    active: false,
  };
}
export function initialTargetSelectionState() {
  return {
    model: undefined as TargetSelectionViewModel | undefined,
    snapshotStatus: { stage: "loading" } as SnapshotStatus,
    cardMotion: { stage: "settled" as const },
  };
}

export function documentKind(value: string): DocumentKind {
  switch (value) {
    case "license":
    case "notice":
      return value;
    default:
      throw new Error(`Invalid About document: ${value}`);
  }
}
