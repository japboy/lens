import fontAwesomeStyles from "@fortawesome/fontawesome-free/css/fontawesome.css?inline";
import fontAwesomeSolidStyles from "@fortawesome/fontawesome-free/css/solid.css?inline";
import { unsafeCSS } from "lit";

export const sharedIconStyles = [unsafeCSS(fontAwesomeStyles), unsafeCSS(fontAwesomeSolidStyles)];
