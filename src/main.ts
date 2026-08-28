import "./styles.css";
import "./lens-app";
import { applyPresentationContext, presentationContextFromSearch } from "./presentation-context";

const appRoot = document.querySelector("lens-app");
if (!(appRoot instanceof HTMLElement)) {
  throw new Error("Lens application root is missing");
}

applyPresentationContext(presentationContextFromSearch(window.location.search), [
  document.documentElement,
  document.body,
  appRoot,
]);
