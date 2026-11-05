// Pure key map for the watchlist. Text fields keep their own keys; only
// Escape leaves them. No DOM access, so node tests load it directly.
(function (root) {
  "use strict";

  const SINGLE = {
    j: "down", ArrowDown: "down", k: "up", ArrowUp: "up", g: "top", G: "bottom",
    Enter: "open", l: "open", ArrowRight: "open", Escape: "back", h: "back", ArrowLeft: "back",
    Tab: "focus", "/": "filter", r: "refresh", R: "refresh-all", y: "copy", o: "console",
    n: "next-error", N: "previous-error", a: "add", e: "edit", J: "move-down", K: "move-up",
    "?": "help", ":": "command", s: "save", ",": "settings",
  };

  function createKeyMap({ now = () => Date.now(), sequenceMs = 600 } = {}) {
    let firstD = null;
    return {
      action(event) {
        if (event.typing) return event.key === "Escape" ? "back" : null;
        if ((event.metaKey || event.ctrlKey) && !event.altKey && String(event.key).toLowerCase() === "k") return "command";
        if (event.metaKey || event.ctrlKey || event.altKey) return null;
        if (event.key === "d") {
          const at = now();
          if (firstD !== null && at - firstD <= sequenceMs) { firstD = null; return "remove"; }
          firstD = at;
          return null;
        }
        firstD = null;
        return Object.hasOwn(SINGLE, event.key) ? SINGLE[event.key] : null;
      },
    };
  }

  const HELP = [
    ["j k", "Move"], ["g G", "Top or bottom"], ["Enter", "Open"], ["Esc", "Back"], ["Tab", "List or inspector"],
    ["/", "Filter"], ["r", "Refresh"], ["R", "Refresh all"], ["y", "Copy"], ["o", "Open in AWS console"],
    ["n N", "Next or previous error"], ["a", "Add a thing"], ["e", "Edit"], ["dd", "Remove"], ["J K", "Move row"],
    [":", "Command line"], ["s", "Save result"], [",", "Settings"], ["?", "All keys"],
  ];

  root.CloudBurritoKeys = { createKeyMap, HELP };
})(typeof window !== "undefined" ? window : globalThis);
