// Reuse existing controls: presentation changes never dispatch AWS work.
(function () {
  "use strict";
  const root = document.documentElement;
  const $ = selector => document.querySelector(selector);
  const theme = $("#theme-toggle");
  const reset = $("#reset-layout-btn");
  const connection = $("#connection-toggle");
  const menu = $("#studio-workspace-menu");
  const summary = $("#studio-workspace-menu-toggle");
  if (!theme || !reset || !connection || !menu || !summary) return;

  const homes = new Map();
  for (const control of [theme, reset, connection]) {
    const anchor = document.createComment(`Original location: ${control.id}`);
    control.before(anchor);
    homes.set(control, anchor);
  }
  root.dataset.runtime = typeof window.__TAURI__ === "undefined" ? "browser" : "desktop";

  function closeMenu(restoreFocus = false) {
    if (!menu.open) return;
    menu.open = false;
    summary.setAttribute("aria-expanded", "false");
    if (restoreFocus) summary.focus({ preventScroll: true });
  }

  function syncPresentation() {
    const studio = root.dataset.design === "studio";
    if (studio) {
      const themeSlot = $("#appearance-theme-slot");
      const resetSlot = $("#studio-reset-slot");
      if (theme.parentElement !== themeSlot) themeSlot.append(theme);
      if (reset.parentElement !== resetSlot) resetSlot.append(reset);
      if (connection.previousElementSibling !== $("#auth-status")) $("#auth-status").after(connection);
    } else {
      closeMenu();
      for (const [control, anchor] of homes) {
        if (control.previousSibling !== anchor) anchor.after(control);
      }
    }
    const themeLabel = `Preview ${root.dataset.theme === "dark" ? "light" : "dark"} theme`;
    theme.textContent = studio ? themeLabel : "◐";
    theme.setAttribute("aria-label", themeLabel);
    reset.textContent = studio ? "Reset layout" : "↺";
    $("#studio-demo-status").hidden = root.dataset.runtime !== "browser";
  }
  syncPresentation();
  new MutationObserver(syncPresentation).observe(root, { attributes: true, attributeFilter: ["data-design", "data-theme"] });

  menu.addEventListener("toggle", () => summary.setAttribute("aria-expanded", String(menu.open)));
  menu.addEventListener("keydown", event => {
    if (event.key !== "Escape" || !menu.open) return;
    event.preventDefault();
    event.stopPropagation();
    closeMenu(true);
  });
  document.addEventListener("pointerdown", event => {
    if (!menu.contains(event.target)) closeMenu(menu.contains(document.activeElement));
  });
  menu.addEventListener("focusout", event => {
    // Pointer focus moves through body before the next control receives focus.
    // Use its destination so the menu does not close before that control's click.
    if (!menu.contains(event.relatedTarget)) closeMenu();
  });
  $("#studio-menu-appearance").addEventListener("click", () => {
    closeMenu(true);
    $("#appearance-btn").click();
  });
  reset.addEventListener("click", () => closeMenu(true));
})();
