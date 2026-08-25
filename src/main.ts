import "./styles.css";
import "./personal-lens-app";

const appView = new URLSearchParams(window.location.search).get("view") === "overlay"
  ? "overlay"
  : "settings";
document.documentElement.dataset.view = appView;
document.body.dataset.view = appView;
