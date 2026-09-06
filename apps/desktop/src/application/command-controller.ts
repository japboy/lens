import type { ReactiveController, ReactiveControllerHost } from "lit";
import {
  canStartCommand,
  IDLE_COMMAND_STATE,
  type CommandIdentity,
  type CommandState,
} from "./command-state";

/** One host-local command generation; late completions cannot overwrite newer work. */
export class CommandController implements ReactiveController {
  state: CommandState = IDLE_COMMAND_STATE;
  private generation = 0;

  constructor(private readonly host: ReactiveControllerHost) {
    host.addController(this);
  }
  hostDisconnected(): void {
    this.generation += 1;
    this.state = IDLE_COMMAND_STATE;
  }

  reportFailure(command: CommandIdentity, message: string): void {
    this.generation += 1;
    this.setState({ stage: "failed", command, message });
  }

  async run(
    command: CommandIdentity,
    action: () => Promise<void | string>,
    successMessage = "",
  ): Promise<void> {
    if (!canStartCommand(this.state, command)) return;
    const generation = ++this.generation;
    this.setState({ stage: "pending", command });
    try {
      const result = await action();
      if (generation !== this.generation) return;
      const message = result || successMessage;
      this.setState(message ? { stage: "succeeded", command, message } : IDLE_COMMAND_STATE);
    } catch (error) {
      if (generation !== this.generation) return;
      this.setState({ stage: "failed", command, message: String(error) });
    }
  }

  private setState(state: CommandState): void {
    this.state = state;
    this.host.requestUpdate();
  }
}
