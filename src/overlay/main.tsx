import React from "react";
import ReactDOM from "react-dom/client";
import Overlay from "./Overlay";
import { disableContextMenu } from "../lib/disableContextMenu";
import "./overlay.css";

disableContextMenu();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <Overlay />
  </React.StrictMode>,
);
