// Development-only: deliberately absent from PAGE_ENTRIES.
import "./styles/document.css";
import "./components/lens-overlay-view";
import "./components/lens-agent-output";
import "./components/lens-session-document";
import type { LensOverlayView } from "./components/lens-overlay-view";
import { mediaCases, mediaFixture, type MediaCase } from "../tests/fixtures/media-parity";
const normal = document.querySelector<LensOverlayView>("#normal")!;
const history = document.querySelector<LensOverlayView>("#history")!;
const select = document.querySelector<HTMLSelectElement>("#case")!;
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
  normal.htmlContent = fixture.htmlContent;
  history.sessionView = {
    revision,
    phase: "ready",
    agent: "codex",
    session_id: `fixture-${choice}`,
    generation: `qualification-${revision}`,
    document: fixture.document,
  };
}
const requested = new URLSearchParams(location.search).get("media");
select.value = mediaCases.includes(requested as MediaCase) ? requested! : "single";
select.addEventListener("change", () => show(select.value as MediaCase));
show(select.value as MediaCase);
