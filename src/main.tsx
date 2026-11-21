import React from "react";
import ReactDOM from "react-dom/client";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);
const label = isTauri() ? getCurrentWindow().label : "main";
if (label.startsWith("screenshot-long-")) {
  void import("./LongScreenshot").then(({LongScreenshot}) => root.render(<LongScreenshot />));
} else if (label.startsWith("screenshot-") && !label.startsWith("screenshot-save-")) {
  void import("./Screenshot").then(({ScreenshotEditor, ScreenshotPin, ScreenshotToast}) => {
    if (label.startsWith("screenshot-toast-")) root.render(<ScreenshotToast />);
    else root.render(label.startsWith("screenshot-pin-") ? <ScreenshotPin /> : <ScreenshotEditor />);
  });
} else if (label === "main") {
  root.render(<React.StrictMode><App /></React.StrictMode>);
}
