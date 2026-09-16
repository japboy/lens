import type { ReactiveController, ReactiveControllerHost } from "lit";
import type { SessionView } from "./session-document";
import type { Unlisten, WebviewPort } from "./webview-port";

/** Subscribe before fetching, and reject stale responses and disconnected generations. */
export class SessionViewController implements ReactiveController {
  view: SessionView | undefined;
  private generation = 0;
  private unlisten: Unlisten | undefined;
  constructor(
    private readonly host: ReactiveControllerHost,
    private readonly port: WebviewPort,
  ) {
    host.addController(this);
  }
  hostConnected(): void {
    void this.load(++this.generation);
  }
  hostDisconnected(): void {
    ++this.generation;
    this.unlisten?.();
    this.unlisten = undefined;
  }
  private apply(view: SessionView, generation: number): void {
    if (generation !== this.generation || (this.view && view.revision <= this.view.revision))
      return;
    this.view = view;
    this.host.requestUpdate();
  }
  private async load(generation: number): Promise<void> {
    try {
      const unlisten = await this.port.subscribeToSessionView((view) =>
        this.apply(view, generation),
      );
      if (generation !== this.generation) {
        unlisten();
        return;
      }
      this.unlisten = unlisten;
      this.apply(await this.port.getSessionView(), generation);
    } catch (error) {
      // Keep an already received authoritative event; connection errors cannot replace it.
      if (generation === this.generation && !this.view) {
        this.view = {
          revision: -1,
          phase: "failed",
          error: `Unable to load session: ${String(error)}`,
        };
        this.host.requestUpdate();
      }
    }
  }
}
