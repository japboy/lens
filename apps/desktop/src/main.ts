import "./styles/document.css";
import { LensApp } from "./lens-app";
import { applyPresentationContext, presentationContextFromSearch } from "./presentation-context";

const appRoot = document.querySelector("lens-app");
if (!(appRoot instanceof LensApp)) {
  throw new Error("Lens application root is missing");
}

const context = presentationContextFromSearch(window.location.search);
appRoot.context = context;
applyPresentationContext(context, [document.documentElement, document.body, appRoot]);
