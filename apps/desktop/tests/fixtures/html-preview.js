import "/src/styles/document.css";
  import "/src/components/lens-overlay-view.ts";
  import "/src/components/lens-agent-output.ts";
  const content = `<!doctype html><html lang="en"><head><style>
    :root { --accent: #7c3aed; font-family: system-ui; color: #172033; }
    body { margin: 0; padding: 28px; background: linear-gradient(125deg,#ede9fe,#cffafe); }
    main { display: grid; grid-template-columns: 1fr 1fr; gap: calc(8px + 1vw); }
    .card { padding: 20px; border-radius: 18px; background: #ffffffc9; box-shadow: 0 8px 30px #17203318; }
    h1 { color: var(--accent); } svg { width: 100%; height: 100px; }
    @media(max-width:480px) { main { grid-template-columns: 1fr; } }
  </style></head><body><h1>HTML / CSS preview</h1><main>
    <section class="card"><h2>Native document</h2><p>Grid, variables, gradients and SVG.</p>
      <svg viewBox="0 0 240 100"><defs><linearGradient id="g"><stop stop-color="#7c3aed"/><stop offset="1" stop-color="#06b6d4"/></linearGradient></defs><rect width="240" height="100" rx="20" fill="url(#g)"/></svg></section>
    <section class="card"><details><summary>Show details</summary><p id="extra">Native disclosure works without JavaScript.</p></details><p><label>Name <input placeholder="Type here"></label></p><a href="#bottom">Jump to bottom</a><p><a href="https://example.com/">Reference</a></p><form action="https://example.invalid/submit"><button>Submit (blocked)</button></form></section>
    </main><div style="height:500px"></div><h2 id="bottom">Bottom anchor</h2>
    <img src="https://example.invalid/network-probe" onerror="parent.postMessage('unexpected-script','*')">
    <script>parent.postMessage('unexpected-script','*')</script>
    </body></html>`;
  const blocks = [{type:"html", resource_id:"fixture-html", mime_type:"text/html", uri:"urn:lens:fixture", byte_length:new TextEncoder().encode(content).length}, {type:"markdown",text:"Static HTML verification. Existing Hero controls remain above the preview."}];
  const view = document.querySelector("lens-overlay-view");
  view.active = true;
  view.htmlContent = {resourceId:"fixture-html", status:"ready", content};
  view.model = {platform:"macos", lens:{operation_id:"html-fixture",stage:"completed", output_blocks:blocks,
    representation:{representation_id:"html-fixture:1",context_id:"fixture",context_revision:1,projection:{revision:1,digest:"sha256:fixture"},run_id:"fixture",output_blocks:blocks}},
    pending:false,cancelPending:false,message:""};
  window.unexpectedScripts = [];
  window.addEventListener("message", event => window.unexpectedScripts.push(event.data));
