// Development-only: deliberately absent from PAGE_ENTRIES.
import "ui/styles/document.css";
import "ui/components/views/lens-overlay-view";
import "ui/components/overlay/lens-agent-output";
import "ui/components/overlay/lens-session-document";
import { ResponseHistoryController } from "ui/resources/response-history-controller";
import type { ReactiveControllerHost } from "lit";
import type { LensOverlayView } from "ui/components/views/lens-overlay-view";
import { mediaCases, mediaFixture, type MediaCase } from "ui/test-fixtures/media-parity";
const normal = document.querySelector<LensOverlayView>("#normal")!;
const history = document.querySelector<LensOverlayView>("#history")!;
const select = document.querySelector<HTMLSelectElement>("#case")!;
const controllers = new Map<LensOverlayView, ResponseHistoryController>();
let revision = 0;
function show(choice: MediaCase) {
  const fixture = mediaFixture(choice);
  normal.active = history.active = true;
  normal.model = {
    platform: "macos",
    lens: { ...fixture.lens, operation_id: `fixture-${++revision}` },
    pending: false,
    cancelPending: false,
    message: "",
  };
  normal.sessionView = {
    revision,
    phase: "live",
    agent: "codex",
    session_id: `live-fixture-${choice}`,
    generation: `live-qualification-${revision}`,
    document: fixture.document,
  };
  history.sessionView = {
    revision,
    phase: "ready",
    agent: "codex",
    session_id: `fixture-${choice}`,
    generation: `qualification-${revision}`,
    document: fixture.document,
    interpretation: fixture.interpretation,
  };
  for (const [view, replay] of [
    [normal, false],
    [history, true],
  ] as const) {
    controllers.get(view)?.hostDisconnected();
    const controller = new ResponseHistoryController(
      {
        addController: () => undefined,
        requestUpdate: () => {
          view.responseHistory = controller.presentation;
        },
      } as unknown as ReactiveControllerHost,
      fixture.port,
      fixture.loadSessionBlock,
    );
    controllers.set(view, controller);
    if (replay) controller.synchronizeHistory(view.sessionView);
    else controller.synchronize(view.model!.lens);
    view.responseHistory = controller.presentation;
    view.loadResponseBlock = controller.loadBlock;
    view.requestResponseMedia = controller.requestMedia;
  }
}
const requested = new URLSearchParams(location.search).get("media");
select.value = mediaCases.includes(requested as MediaCase) ? requested! : "single";
select.addEventListener("change", () => show(select.value as MediaCase));
show(select.value as MediaCase);
