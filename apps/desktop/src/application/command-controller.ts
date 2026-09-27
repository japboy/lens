import type { ReactiveController, ReactiveControllerHost } from "lit";
import {
  canStartCommand,
  commandLane,
  IDLE_COMMAND_STATE,
  type CommandIdentity,
  type CommandLane,
  type CommandState,
} from "./command-state";

/** One host-local command generation; late completions cannot overwrite newer work. */
export class CommandController implements ReactiveController {
  private readonly lanes = new Map<CommandLane, { generation: number; state: CommandState }>();
  private generation = 0;

  get states(): CommandState[] {
    return [...this.lanes.values()].map(({ state }) => state);
  }

  get state(): CommandState {
    const states = this.states;
    return (
      states.find(
        (state) =>
          state.stage === "pending" &&
          state.command.scope === "overlay" &&
          ["close", "pause", "resume"].includes(state.command.type),
      ) ??
      states.find((state) => state.stage === "pending") ??
      states.at(-1) ??
      IDLE_COMMAND_STATE
    );
  }

  constructor(private readonly host: ReactiveControllerHost) {
    host.addController(this);
  }

  hostDisconnected(): void {
    this.generation += 1;
    this.lanes.clear();
  }

  reportFailure(command: CommandIdentity, message: string): void {
    this.setState(commandLane(command), ++this.generation, { stage: "failed", command, message });
  }

  async run(
    command: CommandIdentity,
    action: () => Promise<void | string>,
    successMessage = "",
  ): Promise<void> {
    if (!this.states.every((state) => canStartCommand(state, command))) return;
    const lane = commandLane(command);
    const generation = ++this.generation;
    this.setState(lane, generation, { stage: "pending", command });
    try {
      const result = await action();
      if (this.lanes.get(lane)?.generation !== generation) return;
      const message = result || successMessage;
      this.setState(
        lane,
        generation,
        message ? { stage: "succeeded", command, message } : IDLE_COMMAND_STATE,
      );
    } catch (error) {
      if (this.lanes.get(lane)?.generation !== generation) return;
      this.setState(lane, generation, { stage: "failed", command, message: String(error) });
    }
  }

  private setState(lane: CommandLane, generation: number, state: CommandState): void {
    this.lanes.delete(lane);
    this.lanes.set(lane, { generation, state });
    this.host.requestUpdate();
  }
}
