import { prepareHtmlPreview } from "../html-output";
const host = globalThis as unknown as {
  onmessage: (event: MessageEvent<{ id: number; text: string }>) => void;
  postMessage: (response: unknown) => void;
};
host.onmessage = ({ data }) => {
  try {
    host.postMessage({ id: data.id, value: prepareHtmlPreview(data.text) });
  } catch (error) {
    host.postMessage({ id: data.id, error: String(error) });
  }
};
host.postMessage({ type: "ready" });
