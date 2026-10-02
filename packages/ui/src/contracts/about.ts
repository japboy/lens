export interface AboutInfo {
  name: string;
  version: string;
  copyright: string;
}

export interface AboutDocuments {
  license: string;
  notice: string;
}

export type ReleaseAvailability =
  | { revision: number; stage: "idle" | "checking" | "current" }
  | { revision: number; stage: "failed"; retry_after_epoch_ms?: number }
  | { revision: number; stage: "available"; version: string; release_url: string };
