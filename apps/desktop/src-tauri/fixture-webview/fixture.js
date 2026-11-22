const { invoke } = window.__TAURI__.core;
const buttons = [...document.querySelectorAll("button[data-action]")];
const status = document.querySelector("#status");
const detail = document.querySelector("#detail");
const capture = document.querySelector("#capture");
const stop = document.querySelector("#stop");
const next = document.querySelector("#next");
const marker = document.querySelector("#marker");
let stopping;
let pendingSelection;
let nextRead;

function clearImage() {
  capture.hidden = true;
  capture.removeAttribute("src");
}

function clearMarker() {
  marker.textContent = "";
  marker.hidden = true;
}

async function render(snapshot) {
  if (snapshot.png) {
    capture.src = "data:image/png;base64," + snapshot.png;
    await capture.decode();
    capture.hidden = false;
  } else {
    clearImage();
  }
  status.textContent =
    snapshot.title ??
    (snapshot.stage === "cancelled" ? "Selection cancelled" : "Source unavailable");
  detail.textContent =
    snapshot.error ??
    (snapshot.read
      ? `Published paired UIA content and WGC capture · Read ${snapshot.read.sequence}`
      : "");
  marker.textContent = snapshot.marker ?? "";
  marker.hidden = !snapshot.marker;
  // Wait through a paint before acknowledging the visible image or withdrawal.
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  await invoke("fixture_rendered", {
    rendered: {
      operation: snapshot.operation,
      read: snapshot.read,
      marker: marker.hidden ? null : marker.textContent,
      hasImage: !capture.hidden && capture.complete && capture.naturalWidth > 0,
    },
  });
}

async function select(action) {
  const selection = {
    continuous: ["continuous_a", "worker_exit_a", "stall_refresh_a"].includes(action),
    withdrawnStage: undefined,
  };
  pendingSelection = selection;
  for (const button of buttons) button.disabled = true;
  next.textContent = action === "worker_exit_a" ? "Exit worker" : "Next read";
  if (selection.continuous || action === "stop_a" || action === "late_a")
    void watchAcquisition(selection);
  status.textContent = "Waiting for fixture confirmation";
  detail.textContent = "";
  clearImage();
  clearMarker();
  try {
    const snapshot = await invoke("fixture_select", { action });
    if (selection.withdrawnStage !== snapshot.stage) await render(snapshot);
  } catch (error) {
    clearImage();
    clearMarker();
    status.textContent = "Fixture operation failed";
    detail.textContent = String(error);
  } finally {
    pendingSelection = undefined;
    stopping = undefined;
    stop.disabled = true;
    next.disabled = true;
    next.textContent = "Next read";
    nextRead = undefined;
    for (const button of buttons) button.disabled = false;
  }
}

async function watchAcquisition(selection) {
  let seenSnapshot;
  try {
    while (pendingSelection === selection) {
      if (selection.continuous) {
        const snapshot = await invoke("fixture_continuous_snapshot");
        if (pendingSelection !== selection) return;
        const key = snapshot?.read?.sequence ?? snapshot?.stage;
        if (snapshot && key !== seenSnapshot) {
          await render(snapshot);
          if (pendingSelection !== selection) return;
          seenSnapshot = key;
          nextRead = snapshot.read;
          next.disabled = !nextRead;
          if (!nextRead) selection.withdrawnStage = snapshot.stage;
        }
      }
      const operation = await invoke("fixture_waiting");
      if (pendingSelection !== selection) return;
      if (operation) {
        stopping = operation;
        if (!selection.continuous) status.textContent = "Fixture acquisition in progress";
        else detail.textContent = "Read 3 is waiting · Stop acquisition";
        stop.disabled = false;
        return;
      }
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  } catch {
    stop.disabled = true;
    next.disabled = true;
  }
}

stop.addEventListener("click", async () => {
  const operation = stopping;
  const selection = pendingSelection;
  if (!operation) return;
  stop.disabled = true;
  try {
    const snapshot = await invoke("fixture_stop", { operation });
    if (snapshot && selection?.continuous && pendingSelection === selection) {
      selection.withdrawnStage = snapshot.stage;
      await render(snapshot);
    } else if (!snapshot && stopping === operation) {
      stop.disabled = false;
    }
  } catch (error) {
    if (stopping === operation) {
      detail.textContent = String(error);
      stop.disabled = false;
    }
  }
});

next.addEventListener("click", async () => {
  const read = nextRead;
  if (!read) return;
  next.disabled = true;
  try {
    if (!(await invoke("fixture_advance", { read }))) next.disabled = false;
  } catch (error) {
    detail.textContent = String(error);
    next.disabled = false;
  }
});

for (const button of buttons) button.addEventListener("click", () => select(button.dataset.action));
