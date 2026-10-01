// Trusted click routing only. The native Host validates display ownership and opens HTTP(S).
(() => {
  let sequence = 0;
  document.addEventListener("click", (event) => {
    if (!event.isTrusted || event.defaultPrevented || event.button !== 0) return;
    const anchor = event.target instanceof Element ? event.target.closest("a[href]") : null;
    if (!anchor || anchor.hasAttribute("download")) return;
    let url;
    try {
      const href = anchor.getAttribute("href");
      if (href.trim().startsWith("#")) return;
      url = new URL(href, document.baseURI);
      const current = new URL(document.URL);
      if (
        url.origin === current.origin &&
        url.pathname === current.pathname &&
        url.search === current.search
      )
        return;
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
