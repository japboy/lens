// Identities, dependency roles, capability scopes and variants are separate policy data.
// Changing this control plane requires native verification; it is never skip authority.
export type DependencyKind = "normal" | "dev" | "build";
export type Member = {
  ecosystem: "cargo" | "pnpm";
  name: string;
  directory: string;
  role: "repository" | "application" | "domain" | "usecase" | "port" | "adapter" | "configuration";
  capability: "repository" | "desktop" | "observation" | "platform";
  implementation: "portable" | "common-shell" | "macos" | "tooling" | "webview" | "transport";
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
        "adapter-output-mcp",
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
      dev: ["pretty_assertions", "tauri", "toml"],
      build: ["tauri-build", "serde_json", "toml"],
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
    name: "adapter-output-mcp",
    directory: "packages/adapter-output-mcp",
    role: "adapter",
    capability: "desktop",
    implementation: "transport",
    dependencies: {
      normal: [
        "rmcp",
        "axum",
        "hyper",
        "hyper-util",
        "serde",
        "serde_json",
        "uuid",
        "tokio",
        "tokio-util",
      ],
      dev: ["reqwest", "tokio"],
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

/// Reviewed feature selection for every dependency that narrows or extends the default
/// set. A dependency name alone does not describe what it can do, so the capabilities are
/// pinned at the same granularity the names are: adding one is a policy change, not an
/// unreviewed edit to a manifest.
export const DEPENDENCY_FEATURES = [
  { member: "adapter-output-mcp", kind: "normal", target: null, name: "hyper", default: true, features: ["server", "http1"] },
  { member: "adapter-output-mcp", kind: "normal", target: null, name: "hyper-util", default: true, features: ["tokio", "service"] },
  { member: "adapter-output-mcp", kind: "normal", target: null, name: "rmcp", default: false, features: ["server", "macros", "transport-streamable-http-server"] },
  { member: "adapter-output-mcp", kind: "normal", target: null, name: "serde", default: true, features: ["derive"] },
  { member: "adapter-output-mcp", kind: "normal", target: null, name: "tokio", default: true, features: ["net", "rt", "macros"] },
  { member: "adapter-output-mcp", kind: "normal", target: null, name: "uuid", default: true, features: ["v4", "serde"] },
  { member: "adapter-output-mcp", kind: "dev", target: null, name: "reqwest", default: false, features: ["json"] },
  { member: "adapter-output-mcp", kind: "dev", target: null, name: "tokio", default: true, features: ["time", "io-util"] },
  { member: "adapter-platform-macos", kind: "normal", target: null, name: "serde", default: true, features: ["derive"] },
  { member: "adapter-platform-macos", kind: "normal", target: null, name: "tokio", default: true, features: ["sync"] },
  { member: "adapter-platform-macos", kind: "normal", target: null, name: "uuid", default: true, features: ["v4", "serde"] },
  { member: "desktop", kind: "normal", target: null, name: "reqwest", default: true, features: ["json", "stream"] },
  { member: "desktop", kind: "normal", target: null, name: "serde", default: true, features: ["derive"] },
  { member: "desktop", kind: "normal", target: null, name: "tauri", default: false, features: ["image-png", "macos-private-api", "tray-icon", "compression", "common-controls-v6", "dynamic-acl", "x11", "dbus"] },
  { member: "desktop", kind: "normal", target: 'cfg(target_os = "macos")', name: "tauri", default: true, features: ["macos-private-api"] },
  { member: "desktop", kind: "normal", target: null, name: "tokio", default: true, features: ["fs", "io-util", "macros", "process", "sync", "time"] },
  { member: "desktop", kind: "normal", target: null, name: "uuid", default: true, features: ["v4", "serde"] },
  { member: "desktop", kind: "dev", target: null, name: "tauri", default: false, features: ["test"] },
  { member: "domain", kind: "normal", target: null, name: "serde", default: true, features: ["derive"] },
  { member: "domain", kind: "normal", target: null, name: "uuid", default: true, features: ["v4", "serde"] },
  { member: "port-platform", kind: "normal", target: null, name: "serde", default: true, features: ["derive"] },
  { member: "port-platform", kind: "normal", target: null, name: "uuid", default: true, features: ["v4", "serde"] },
  { member: "usecase", kind: "normal", target: null, name: "agent-client-protocol-schema", default: true, features: ["tracing"] },
  { member: "usecase", kind: "normal", target: null, name: "serde", default: true, features: ["derive"] },
  { member: "usecase", kind: "normal", target: null, name: "uuid", default: true, features: ["v4", "serde"] },
  { member: "usecase", kind: "dev", target: null, name: "tokio", default: true, features: ["macros", "rt"] },
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

const common = ["domain", "port-platform", "usecase", "desktop", "adapter-output-mcp"];
const portable = ["domain", "port-platform", "usecase", "adapter-output-mcp"];
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
