import React from "react";
import ReactDOM from "react-dom/client";

import Panel from "./Panel";
import "./i18n";
import { disableNativeContextMenu } from "./disableNativeContextMenu";

disableNativeContextMenu();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Panel />
  </React.StrictMode>,
);
