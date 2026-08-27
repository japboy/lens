import "./styles.css";
import "./personal-lens-app";
import { applyPresentationContext, presentationContextFromSearch } from "./presentation-context";

const appRoot = document.querySelector("personal-lens-app");
if (!(appRoot instanceof HTMLElement)) {
  throw new Error("PersonalLens application root is missing");
}

applyPresentationContext(presentationContextFromSearch(window.location.search), [
  document.documentElement,
  document.body,
  appRoot,
]);
