import { startPage } from "./start-page";

void startPage("about", () => import("../pages/about-page"));
