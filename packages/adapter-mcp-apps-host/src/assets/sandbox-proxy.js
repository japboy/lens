// Trusted document on a dedicated origin, never a host-native IPC surface.
const settings = JSON.parse(document.getElementById("proxy-settings").textContent);
const RESOURCE_READY = "ui/notifications/sandbox-resource-ready";
let phase = "waiting";
let frame;
let teardownId;
const resourceLoad = new AbortController();
function bounded(data) {
  try {
    return (
      data?.jsonrpc === "2.0" &&
      (typeof data.method === "string" || (data.method === undefined && data.id !== undefined)) &&
      new TextEncoder().encode(JSON.stringify(data)).byteLength <= 4 * 1024 * 1024
    );
  } catch {
    return false;
  }
}
function reserved(data) {
  return typeof data.method === "string" && data.method.startsWith("ui/notifications/sandbox-");
}
window.addEventListener("message", async (event) => {
  const fromHost = event.source === parent && event.origin === settings.hostOrigin;
  const fromView = frame && event.source === frame.contentWindow && event.origin === "null";
  if ((!fromHost && !fromView) || !bounded(event.data)) return;
  const message = event.data;
  if (fromHost) {
    if (message.method === "ui/resource-teardown" && phase !== "mounted") {
      phase = "closed";
      resourceLoad.abort();
      frame?.remove();
      parent.postMessage(
        { jsonrpc: "2.0", id: message.id, result: {} },
        settings.hostOrigin === "null" ? "*" : settings.hostOrigin,
      );
      return;
    }
    if (message.method === RESOURCE_READY && phase === "waiting") {
      phase = "loading";
      try {
        if (typeof message.params?.html !== "string") throw new Error("Invalid App resource");
        const response = await fetch(settings.viewUrl, {
          method: "POST",
          headers: { "content-type": "text/html" },
          body: message.params.html,
          signal: resourceLoad.signal,
        });
        if (!response.ok || phase !== "loading") throw new Error("App resource unavailable");
        frame = document.createElement("iframe");
        frame.title = "Interactive Interpretation";
        frame.setAttribute("sandbox", "allow-scripts allow-forms");
        frame.setAttribute("referrerpolicy", "no-referrer");
        frame.style.cssText = "border:0;width:100%;height:100%;display:block";
        document.body.append(frame);
        frame.src = settings.viewUrl;
        phase = "mounted";
      } catch {
        if (phase === "closed") return;
        phase = "failed";
        document.body.textContent = "App resource unavailable.";
      }
      return;
    }
    if (reserved(message) || phase !== "mounted") return;
    frame.contentWindow.postMessage(message, "*");
    if (message.method === "ui/resource-teardown") {
      phase = "closing";
      teardownId = message.id;
    }
  } else {
    if (
      reserved(message) ||
      (phase !== "mounted" &&
        !(phase === "closing" && message.method === undefined && message.id === teardownId))
    )
      return;
    parent.postMessage(message, settings.hostOrigin === "null" ? "*" : settings.hostOrigin);
    if (phase === "closing") {
      phase = "closed";
      frame.remove();
      frame = undefined;
    }
  }
});
if (self === top || new URL(location.href).origin === settings.hostOrigin)
  throw new Error("Invalid sandbox origin");
try {
  void top.document;
  throw new Error("Invalid sandbox isolation");
} catch (error) {
  if (error.name !== "SecurityError") throw error;
}
parent.postMessage(
  { jsonrpc: "2.0", method: "ui/notifications/sandbox-proxy-ready", params: {} },
  settings.hostOrigin === "null" ? "*" : settings.hostOrigin,
);
