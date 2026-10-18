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
              "履歴一覧の日本語表示と、長いセッションタイトル・作業ディレクトリの折り返しを確認する",
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
