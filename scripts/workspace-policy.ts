// Identities, dependency roles, capability scopes and variants are separate policy data.
// Changing this control plane requires native verification; it is never skip authority.
export type DependencyKind = "normal" | "dev" | "build";
export type Member = {
  ecosystem: "cargo" | "pnpm";
  name: string;
  directory: string;
  role: "repository" | "application" | "domain" | "usecase" | "port" | "adapter" | "configuration";
  capability: "repository" | "desktop" | "observation" | "platform";
  implementation: "portable" | "common-shell" | "macos" | "tooling" | "webview";
  dependencies: Partial<Record<DependencyKind, readonly string[]>>;
};

export const MEMBERS: readonly Member[] = [
  {
    ecosystem: "pnpm",
    name: "repo",
    directory: ".",
    role: "repository",
    capability: "repository",
    implementation: "tooling",
    dependencies: { dev: ["typescript-config"] },
  },
  {
    ecosystem: "pnpm",
    name: "desktop",
    directory: "apps/desktop",
    role: "application",
    capability: "desktop",
    implementation: "webview",
    dependencies: { dev: ["typescript-config"] },
  },
  {
    ecosystem: "pnpm",
    name: "typescript-config",
    directory: "packages/typescript-config",
    role: "configuration",
    capability: "repository",
    implementation: "tooling",
    dependencies: {},
  },
  {
    ecosystem: "cargo",
    name: "desktop",
    directory: "apps/desktop/src-tauri",
    role: "application",
    capability: "desktop",
    implementation: "common-shell",
    dependencies: {
      normal: [
        "usecase",
        "port-platform",
        "agent-client-protocol",
        "base64",
        "dirs",
        "flate2",
        "reqwest",
        "serde",
        "serde_json",
        "serde_json_canonicalizer",
        "sha2",
        "tar",
        "tauri",
        "tauri-plugin-dialog",
        "tauri-plugin-opener",
        "thiserror",
        "tokio",
        "uuid",
      ],
      dev: ["pretty_assertions", "tauri"],
      build: ["tauri-build"],
    },
  },
  {
    ecosystem: "cargo",
    name: "domain",
    directory: "packages/domain",
    role: "domain",
    capability: "observation",
    implementation: "portable",
    dependencies: {
      normal: [
        "base64",
        "serde",
        "serde_json",
        "serde_json_canonicalizer",
        "sha2",
        "thiserror",
        "uuid",
      ],
      dev: ["pretty_assertions"],
    },
  },
  {
    ecosystem: "cargo",
    name: "usecase",
    directory: "packages/usecase",
    role: "usecase",
    capability: "observation",
    implementation: "portable",
    dependencies: {
      normal: [
        "agent-client-protocol-schema",
        "domain",
        "port-platform",
        "base64",
        "uuid",
        "serde",
        "serde_json",
        "thiserror",
        "url",
      ],
      dev: ["tokio"],
    },
  },
  {
    ecosystem: "cargo",
    name: "port-platform",
    directory: "packages/port-platform",
    role: "port",
    capability: "platform",
    implementation: "portable",
    dependencies: {
      normal: ["serde", "thiserror", "uuid"],
      dev: ["serde_json"],
    },
  },
  {
    ecosystem: "cargo",
    name: "adapter-platform-macos",
    directory: "packages/adapter-platform-macos",
    role: "adapter",
    capability: "platform",
    implementation: "macos",
    dependencies: {
      normal: ["port-platform", "base64", "serde", "serde_json", "uuid", "tokio"],
      build: ["cc"],
    },
  },
];

export const TARGET_DEPENDENCIES = [
  {
    member: "desktop",
    target: 'cfg(target_os = "macos")',
    kind: "normal",
    name: "adapter-platform-macos",
  },
  { member: "desktop", target: 'cfg(target_os = "macos")', kind: "normal", name: "tauri" },
] as const;

// These are separate production trust domains, not development workspace members.
export const MANAGED_RUNTIME_DIRECTORIES = [
  "apps/desktop/src-tauri/agent-runtime/claude",
  "apps/desktop/src-tauri/agent-runtime/codex",
] as const;

export type BuildVariant = {
  id: string;
  target: "x86_64-unknown-linux-gnu" | "aarch64-apple-darwin";
  packages: readonly string[];
  operation: "check" | "test" | "build";
  profile: "dev" | "test" | "release";
  defaultFeatures: boolean;
  features: readonly string[];
  targets: "lib" | "lib-and-bins";
};

const common = ["domain", "port-platform", "usecase", "desktop"];
const portable = ["domain", "port-platform", "usecase"];
const native = [...common, "adapter-platform-macos"];

export const BUILD_VARIANTS: readonly BuildVariant[] = [
  {
    id: "linux-common-check",
    target: "x86_64-unknown-linux-gnu",
    packages: common,
    operation: "check",
    profile: "dev",
    defaultFeatures: false,
    features: [],
    targets: "lib",
  },
  {
    id: "linux-common-test",
    target: "x86_64-unknown-linux-gnu",
    packages: common,
    operation: "test",
    profile: "test",
    defaultFeatures: false,
    features: [],
    targets: "lib",
  },
  {
    id: "apple-portable-check",
    target: "aarch64-apple-darwin",
    packages: portable,
    operation: "check",
    profile: "dev",
    defaultFeatures: true,
    features: [],
    targets: "lib",
  },
  {
    id: "macos-production-check",
    target: "aarch64-apple-darwin",
    packages: native,
    operation: "check",
    profile: "dev",
    defaultFeatures: true,
    features: [],
    targets: "lib-and-bins",
  },
  {
    id: "macos-production-release",
    target: "aarch64-apple-darwin",
    packages: native,
    operation: "check",
    profile: "release",
    defaultFeatures: true,
    features: [],
    targets: "lib-and-bins",
  },
  {
    id: "macos-test",
    target: "aarch64-apple-darwin",
    packages: native,
    operation: "test",
    profile: "test",
    defaultFeatures: true,
    features: [],
    targets: "lib-and-bins",
  },
  {
    id: "macos-bundle-build",
    target: "aarch64-apple-darwin",
    packages: native,
    operation: "build",
    profile: "release",
    defaultFeatures: true,
    features: ["tauri/custom-protocol"],
    targets: "lib-and-bins",
  },
];

export function variantArguments(variant: BuildVariant): string[] {
  return [
    variant.operation,
    "--locked",
    "--target",
    variant.target,
    ...variant.packages.flatMap((name) => ["--package", name]),
    "--lib",
    ...(variant.targets === "lib-and-bins" ? ["--bins"] : []),
    "--profile",
    variant.profile,
    ...(!variant.defaultFeatures ? ["--no-default-features"] : []),
    ...(variant.features.length ? ["--features", variant.features.join(",")] : []),
  ];
}
