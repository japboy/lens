// Authored unsafe boundaries: conditions and evidence describe the retained effect,
// not a claim that passing Miri establishes native correctness or soundness.
export const MIRI_EXECUTION = {
  toolchain: "nightly-2026-10-17",
  target: "aarch64-apple-darwin",
  seed: 1,
  timeoutMs: 120_000,
} as const;

export const MIRI_GROUPS = {
  presentation: {
    package: "adapter-platform-macos",
    filter: "presentation::tests::",
    tests: [
      "presentation::tests::menu_appearance_preserves_nested_positions_and_optional_images",
      "presentation::tests::menu_appearance_rejects_unbounded_trees_and_images",
      "presentation::tests::completion_receipt_and_wait_are_send",
      "presentation::tests::immediate_completion_preserves_success_and_failure",
      "presentation::tests::refusal_retains_operation_error",
      "presentation::tests::delayed_callback_remains_owner_after_receipt_is_dropped",
      "presentation::tests::null_callback_context_is_ignored",
      "presentation::tests::callback_loss_is_distinct_from_native_failure",
      "presentation::tests::pending_receipt_completes_only_when_callback_arrives",
    ],
  },
  "ax-ownership": {
    package: "adapter-platform-macos",
    filter: "accessibility::tests::miri_boundary_",
    tests: [
      "accessibility::tests::miri_boundary_callback_borrows_live_json_and_context",
      "accessibility::tests::miri_boundary_json_valid_and_malformed_release_once",
      "accessibility::tests::miri_boundary_json_null_is_rejected_without_release",
      "accessibility::tests::miri_boundary_observation_close_and_drop_stop_before_context_release",
      "accessibility::tests::miri_boundary_observation_start_failure_stops_before_context_release",
      "accessibility::tests::miri_boundary_observation_cancel_drop_stops_before_context_release",
    ],
  },
} as const;

type Boundary = {
  owners: readonly string[];
  conditions: readonly string[];
  normalTests: readonly string[];
  miriGroups: readonly (keyof typeof MIRI_GROUPS)[];
  nativeGap: string;
};
const environment = "apps/desktop/src-tauri/src/agent_environment.rs";
const presentation = "packages/adapter-platform-macos/src/presentation.rs";
const shellPresentation = "apps/desktop/src-tauri/src/native/presentation.rs";
const accessibility = "packages/adapter-platform-macos/src/accessibility.rs";
const nativeFixtures = "node scripts/test-native-fixtures.ts";
const nativeBoundaryLifecycle =
  "cargo test --locked -p adapter-platform-macos --test native_boundary_lifecycle --target aarch64-apple-darwin";

export const NATIVE_ONLY_TESTS = [
  {
    package: "adapter-platform-macos",
    target: "native_boundary_lifecycle",
    command: nativeBoundaryLifecycle,
    boundaries: ["ax-ffi", "ax-observer", "native-json"],
    reason:
      "The harness-free main-thread integration uses actual CF/dispatch, native JSON allocation/free and observer error replies through the safe Rust public API. Miri cannot interpret these foreign effects; --lib intentionally selects only Rust ownership tests.",
  },
] as const;

export const RUST_SAFETY_BOUNDARIES = {
  "process-cwd": {
    owners: [environment],
    conditions: [
      "The verified directory descriptor stays owned until exec.",
      "The pre_exec closure performs only async-signal-safe fchdir and error conversion.",
    ],
    normalTests: [
      "cargo test --locked -p desktop --lib agent_environment::tests::snapshot_rejects_replaced_directory",
      "cargo test --locked -p desktop --lib agent_launch::tests::command_keeps_resolved_directory_when_symlink_changes_before_spawn",
    ],
    miriGroups: [],
    nativeGap:
      "Miri cannot exercise the real fork/exec descriptor binding and filesystem race; the normal directory-identity regression is required.",
  },
  "process-session": {
    owners: [environment],
    conditions: [
      "setsid runs only between fork and exec with no allocation or locking.",
      "The child owns its new session/process group and cancellation reaps it.",
    ],
    normalTests: ["cargo test --locked -p desktop --lib agent_environment::tests"],
    miriGroups: [],
    nativeGap:
      "Actual sessions, controlling terminals, child exec and group cancellation require OS process tests.",
  },
  "process-account-temp": {
    owners: [environment],
    conditions: [
      "Only the macOS confstr selector is used.",
      "The writable buffer has its declared length; returned size and terminal NUL are checked before ownership conversion.",
    ],
    normalTests: [
      "cargo test --locked -p desktop --lib agent_environment::tests::shell_acquisition_is_fresh_cwd_sensitive_and_keeps_logical_pwd",
    ],
    miriGroups: [],
    nativeGap:
      "The macOS account temp-directory service is an external system effect; Miri does not verify the OS value or confstr implementation.",
  },
  "process-terminal-test": {
    owners: [environment],
    conditions: [
      "Fixture descriptors remain owned through spawn.",
      "Only terminal/session syscalls run in pre_exec; the fixture acquires a controlling terminal deliberately.",
    ],
    normalTests: [
      "cargo test --locked -p desktop --lib agent_environment::tests::shell_acquisition_from_a_controlling_terminal",
    ],
    miriGroups: [],
    nativeGap:
      "A real controlling terminal is the regression input; replacing it in Miri would omit the essential behavior.",
  },
  "appkit-presentation": {
    owners: [presentation, shellPresentation],
    conditions: [
      "Tauri initializes NSApplication and dispatches calls on AppKit's main thread.",
      "Borrowed NSMenu/NSWindow/WKWebView handles remain live for synchronous use; UTF-8 strings remain owned.",
      "Native validation preserves menu positions, destructive confirmation defaults, appearance and clipping.",
    ],
    normalTests: [
      nativeFixtures,
      "cargo test --locked -p adapter-platform-macos --lib presentation::tests::menu_appearance",
    ],
    miriGroups: [],
    nativeGap:
      "AppKit/menu/dialog/appearance effects require native fixtures or interactive acceptance; these Miri groups do not execute native UI, colors or clipping.",
  },
  "appkit-transition": {
    owners: [presentation, shellPresentation],
    conditions: [
      "Accepted transitions transfer one Box context to exactly one terminal callback; synchronous refusal retains Rust ownership.",
      "Native retains the NSWindow until completion; dropping a Rust receiver does not free callback-owned context.",
      "A null callback context is a no-op.",
    ],
    normalTests: ["cargo test --locked -p adapter-platform-macos --lib presentation::tests"],
    miriGroups: ["presentation"],
    nativeGap:
      "Miri checks the production Rust start/refusal/callback/receipt paths with a deterministic start effect; actual AppKit animation, handle retention and callback scheduling still require native acceptance.",
  },
  "ax-ffi": {
    owners: [accessibility],
    conditions: [
      "Native owns the operation/window/process metadata and the promoted AXWindow identity.",
      "CString inputs and initialized geometry outputs remain live through synchronous calls.",
      "Current geometry uses one main-thread transaction; only never-promoted sources may fall back to SCWindow geometry.",
    ],
    normalTests: [
      nativeFixtures,
      nativeBoundaryLifecycle,
      "cargo test --locked -p adapter-platform-macos --lib accessibility::tests::current_frame_fallback",
    ],
    miriGroups: [],
    nativeGap:
      "AX permissions, CFEqual graph identity, bounded ranged reads, native string allocation and run-loop effects are external to Miri and retain native fixture obligations.",
  },
  "ax-observer": {
    owners: [accessibility],
    conditions: [
      "Native only borrows the registration-owned stable context during synchronous callbacks.",
      "Callback JSON is NUL-terminated and live for delivery; every authority field must match.",
      "A started handshake establishes cleanup ownership before decoding; synchronous stop clears native callback/refcon before context release.",
      "Stop passes the still-live Arc::as_ptr as its exact receipt; native compares observationContext on the main thread before touching the current observer, so reused IDs or epochs cannot authorize a stale receipt.",
      "The Arc stays owned through synchronous stop, preventing its allocation address from being reused while that receipt is live.",
    ],
    normalTests: [
      nativeFixtures,
      nativeBoundaryLifecycle,
      "cargo test --locked -p adapter-platform-macos --lib accessibility::tests",
    ],
    miriGroups: ["ax-ownership"],
    nativeGap:
      "Miri runs the production callback and registration cleanup with only the external stop effect supplied; native fixtures must verify exact context-receipt matching, rejection of stale receipts after ID/epoch reuse without stopping the new owner, AXObserver stop/refcon clearing and real run-loop delivery.",
  },
  "native-json": {
    owners: [accessibility, presentation],
    conditions: [
      "A non-null native response is a live NUL-terminated allocation until its matching deallocator.",
      "Ownership is released exactly once on success, parse error and early return; null is rejected without release.",
      "Callback JSON is borrowed only during delivery and never released by Rust.",
    ],
    normalTests: [
      nativeFixtures,
      nativeBoundaryLifecycle,
      "cargo test --locked -p adapter-platform-macos --lib accessibility::tests",
    ],
    miriGroups: ["ax-ownership"],
    nativeGap:
      "Miri exercises production NativeJson parse/Drop with an owned CString and a matching injected free effect; it cannot validate the real AX/NSDateFormatter allocator or native output contract.",
  },
} as const satisfies Record<string, Boundary>;

// No first-party Miri exclusions are admitted. A future exclusion must add its
// exact source, test and safety justification here and receive boundary review.
export const MIRI_EXCLUSIONS: readonly { owner: string; test: string; reason: string }[] = [];
