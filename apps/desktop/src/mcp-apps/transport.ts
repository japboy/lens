import { PostMessageTransport } from "@modelcontextprotocol/ext-apps/app-bridge";

export const MAX_APP_MESSAGE_BYTES = 4 * 1024 * 1024;
type RpcMessage = Parameters<PostMessageTransport["send"]>[0];

export function isBoundedRpcMessage(value: unknown): value is RpcMessage {
  if (!value || typeof value !== "object") return false;
  const message = value as Record<string, unknown>;
  if (message.jsonrpc !== "2.0") return false;
  if (message.method !== undefined && typeof message.method !== "string") return false;
  if (
    message.method === undefined &&
    (message.id === undefined || (message.result === undefined && message.error === undefined))
  )
    return false;
  if (message.id !== undefined && typeof message.id !== "string" && typeof message.id !== "number")
    return false;
  try {
    return new TextEncoder().encode(JSON.stringify(value)).byteLength <= MAX_APP_MESSAGE_BYTES;
  } catch {
    return false;
  }
}

/** PostMessageTransport with explicit origin admission and no payload logging. */
export class OriginBoundAppTransport extends PostMessageTransport {
  private listening = false;
  private closed = false;
  constructor(
    private readonly target: Window,
    private readonly origin: string,
    private readonly owner: Window,
  ) {
    super(target, target);
  }
  private readonly receive = (event: MessageEvent): void => {
    if (
      this.closed ||
      event.source !== this.target ||
      event.origin !== this.origin ||
      !isBoundedRpcMessage(event.data)
    )
      return;
    this.onmessage?.(event.data);
  };
  override async start(): Promise<void> {
    if (this.closed) throw new Error("App transport is closed");
    if (!this.listening) this.owner.addEventListener("message", this.receive);
    this.listening = true;
  }
  override async send(message: RpcMessage): Promise<void> {
    if (this.closed) throw new Error("App transport is closed");
    this.target.postMessage(message, this.origin);
  }
  override async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    this.owner.removeEventListener("message", this.receive);
    this.listening = false;
    this.onclose?.();
  }
}
