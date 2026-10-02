import type { AgentSessionControlState } from "../../src/contracts/lens";
export function sessionControlFixture(): AgentSessionControlState {
  return {
    instance_id: "instance",
    operation_id: "operation",
    session_id: "session",
    agent_name: "Synthetic Agent",
    active: true,
    config_revision: 4,
    agent_default: "safe",
    effective_mode: "safe",
    configured_mode: "safe",
    modes: [],
    config_options: [
      {
        id: "model",
        name: "Agent Model",
        type: "select",
        category: "model",
        currentValue: "second",
        options: [
          {
            group: "z",
            name: "Agent Group",
            options: [
              { value: "second", name: "Second" },
              { value: "first", name: "First" },
            ],
          },
        ],
      },
    ],
    interactions: [],
  };
}
