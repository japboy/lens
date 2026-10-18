import type { HistoryPresentation } from "ui/contracts/view-models";

/** Synthetic presentation only: preview intents never execute native history operations. */
export function historyFixture(scenario: string): HistoryPresentation {
  if (scenario === "history-loading") {
    return { stage: "loading", offset: 0, operation: "synchronizing" };
  }
  const entries =
    scenario === "history-empty"
      ? []
      : [
          {
            id: "fixture-codex",
            title: "Review session history management",
            agent: "Codex",
            cwd: "/Projects/lens",
            updated_at: "2026-10-18T03:20:00Z",
            state: "listed" as const,
            available: true,
          },
          {
            id: "fixture-claude",
            title:
              "\u5c65\u6b74\u4e00\u89a7\u306e\u65e5\u672c\u8a9e\u8868\u793a\u3068\u3001\u9577\u3044\u30bb\u30c3\u30b7\u30e7\u30f3\u30bf\u30a4\u30c8\u30eb\u30fb\u4f5c\u696d\u30c7\u30a3\u30ec\u30af\u30c8\u30ea\u306e\u6298\u308a\u8fd4\u3057\u3092\u78ba\u8a8d\u3059\u308b",
            agent: "Claude",
            cwd: "/Projects/a-project-with-a-long-working-directory-name/packages/desktop",
            updated_at: "2026-10-17T14:05:00Z",
            state: "not_seen" as const,
            available: true,
          },
          {
            id: "fixture-previous",
            title: "Previously configured Agent session",
            agent: "Previous external Agent",
            cwd: "/Projects/archive",
            updated_at: null,
            state: "local_only" as const,
            available: false,
          },
        ];
  return {
    stage: "ready",
    offset: 0,
    page: { revision: 42, total: entries.length, entries },
    operation: "idle",
    ...(scenario === "history-partial"
      ? {
          outcome: {
            kind: "sync" as const,
            result: {
              succeeded: 2,
              failed: 1,
              purged: 1,
              errors: [
                "Claude · /Projects/archive: Agent connection timed out; cached history was retained",
              ],
            },
          },
        }
      : {}),
  };
}
