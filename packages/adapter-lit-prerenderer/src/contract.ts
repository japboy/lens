import type { PluginOption } from "vite";
export type PrerenderPage = {
  html: string;
  rootTag: string;
  outlet: string;
  nestedRootTags?: readonly string[];
};
export type VerificationContract = {
  pages: Readonly<Record<string, PrerenderPage>>;
  resources: (directory: string) => readonly { namespace: string; paths: readonly string[] }[];
};
export type PrerenderContract = {
  applicationPath: string;
  clientPath: string;
  stagingRoot: string;
  renderEntry: string;
  rendererBundleFilename: string;
  pages: Readonly<Record<string, PrerenderPage>>;
  sourceInputs: () => Map<string, Buffer>;
  linkDependencies: (staging: string) => Promise<void>;
  plugins: (applicationRoot: string) => Promise<PluginOption[]>;
  verify: (directory: string, generation: string) => void;
  assertOutput: (output: string) => void;
};
