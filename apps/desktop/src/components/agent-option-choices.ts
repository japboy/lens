import type { SessionConfigOption } from "../types";
import type { SelectOption } from "./lens-select";

export function agentOptionChoices(option: SessionConfigOption): SelectOption[] {
  return (option.options ?? []).flatMap((choice) =>
    "group" in choice
      ? choice.options.map((item) => ({
          value: item.value,
          label: item.name,
          description: item.description,
          group: choice.name,
        }))
      : [{ value: choice.value, label: choice.name, description: choice.description }],
  );
}
