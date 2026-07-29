/** Suppresses the native webview right-click menu (e.g. "Inspect Element") app-wide. */
export function disableContextMenu() {
  window.addEventListener("contextmenu", (e) => e.preventDefault());
}
