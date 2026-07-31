import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./i18n";
import { disableNativeContextMenu } from "./disableNativeContextMenu";
import { ensureNotificationPermission } from "./notifications";

// Prime OS notification permission for the whole app (covers Rust-fired notifications).
void ensureNotificationPermission();
disableNativeContextMenu();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
