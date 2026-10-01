// Presentation helper only. Native opening still requires the trusted Lens control.
(() => {
  let sequence = 0;
  document.addEventListener("click", (event) => {
    if (!event.isTrusted || event.defaultPrevented || event.button !== 0) return;
    const anchor = event.target instanceof Element ? event.target.closest("a[href]") : null;
    if (!anchor || anchor.hasAttribute("download")) return;
    let url;
    try {
      url = new URL(anchor.getAttribute("href"));
    } catch {
      return;
    }
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) return;
    event.preventDefault();
    parent.postMessage(
      {
        jsonrpc: "2.0",
        id: `lens-link-${++sequence}`,
        method: "ui/open-link",
        params: { url: url.href },
      },
      "*",
    );
  });
})();
