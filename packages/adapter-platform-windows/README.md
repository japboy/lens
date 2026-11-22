# Cooperative Win32 source adapter

This development adapter implements the existing selection, accessibility, image capture, observation and accessibility-readiness ports for privately launched Win32 fixtures. It follows the approved scope in [#207](https://github.com/japboy/lens/issues/207#issuecomment-7029089740) and exercises the shared acquisition path for [#209](https://github.com/japboy/lens/issues/209).

The fixture reserves its owned window on its UI thread. A private worker binds the retained UIA root and WGC item to that reservation. The user confirms the fixture before WGC starts. The owner validates the reservation before each binding phase and acknowledges the first publication. The validation host passes the correlated extraction and PNG through the shared context, media and projection code and accepts the read with the common admission ledger. Native IDs stay inside the adapter.

The scope is Windows 11 x64, an interactive desktop, 96 DPI and one registered fixture with stable geometry. The default is a single read. The continuous validation mode allows up to three sequential reads under one operation and receipt, retaining the owner reservation, worker channel, UIA root, WGC session and observer. A read must use the next sequence, and its paired extraction and image are consumed only by that sequence. Each publication requires a new owner check and an outstanding common read. The owner makes a fixed, non-renewing 180-second reservation. This covers the selection deadline, both pauses between reads, the read and publication deadlines, and cleanup. The owner process has a separate 210-second lifetime limit.

A timeout, invalid reservation, mismatched native response, unavailable retained object or observed destruction invalidates the registration. The observation port delivers native destruction when observed. Closure of the private channel without a destruction record closes the stream. The host closes admission, withdraws the current publication and releases the helpers on loss or Stop. A new selection starts a new operation, owner, worker and receipt. The readiness port describes the retained fixture without prompting.

## Run the Windows validation

Use an interactive Windows 11 x64 session with MSVC, the Windows SDK and the repository Rust toolchain. Build the three helpers from this checkout. Supply their directory explicitly and choose a fresh evidence destination.

```powershell
& ./packages/adapter-platform-windows/fixtures/build.ps1 -OutputDirectory .git/lens-fixture-binaries
cargo run --locked -p adapter-platform-windows --example cooperative_validation -- .git/lens-fixture-binaries .git/lens-fixture-matrix matrix
cargo run --locked -p adapter-platform-windows --example cooperative_validation -- .git/lens-fixture-binaries .git/lens-fixture-cancel cancel
cargo run --locked -p adapter-platform-windows --example cooperative_validation -- .git/lens-fixture-binaries .git/lens-fixture-continuous continuous
```

In matrix mode, confirm A, a second A, and B. The second case terminates its harness-owned fixture after the shared context is assembled. The executable checks rejection of that result, rejection of the previous receipt after selecting B, and one publication per accepted read. In cancel mode, dismiss the A confirmation with Cancel. Standard output records publication verdicts, standard error records native child events, and accepted contexts and cropped PNGs are written to the evidence directory.

In continuous mode, confirm A and then B. The harness changes the fixture text and bitmap on the second read and verifies both through the shared context path. It collects A's third result and then destroys A before committing it. Once the correlated native observation arrives, it withdraws A's publication, rejects the held result and requires explicit selection of B. After B's first publication, it invokes Stop at a controlled wait on B's second read, then releases the real native result to check that publication still rejects it.

## Run the Tauri Desktop development window

The opt-in `windows-fixture-desktop` feature starts the Desktop binary with an isolated, embedded WebView2 page only on Windows x64 MSVC with a debug profile and debug assertions. Other targets and release profiles reject the feature. No default feature activates this route. The Desktop host uses the shared initial-publication validation, media transfer checks and read admission ledger. It keeps its window and data directory separate from the regular Lens session. The standalone adapter example retains its Windows x64 compilation scope; the native helper build requires MSVC.

```powershell
& ./packages/adapter-platform-windows/fixtures/build.ps1 -OutputDirectory .git/lens-fixture-binaries
cargo run --locked -p desktop --features windows-fixture-desktop --bin lens -- .git/lens-fixture-binaries .git/lens-desktop-evidence
```

Select A and confirm. Then use **Close A before publication** and confirm to exercise owner loss after context assembly. **Reselect fixture B** tests old-result rejection and publishes B's new context. Finally, open a new selection and cancel in the native dialog. The WebView acknowledges each rendered result after decoding any image. Desktop verdicts go to standard output, native helper records go to standard error, and admitted contexts and PNGs go to the fresh evidence directory.

For one-shot acquisition fault checks, **Stall read** enables **Stop acquisition** after the worker reaches its controlled wait. **Deadline** uses a two-second fixture deadline. **Late result** lets the original worker deliver a result after Stop has closed admission.

The fixture page also provides continuous reads, explicit Stop with a held late result, worker-exit invalidation, and stalled-refresh deadline checks. Renderer receipts correlate the operation, read, marker and decoded image state; Stop withdraws the old view before the held result is delivered and rejected. These are opt-in development scenarios, not normal product refresh. The regular Desktop shell, Agent setup, arbitrary Windows applications and packaging remain outside this scope. Managed runtime artifact policy and normal macOS entry points are unchanged.
