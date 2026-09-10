// Local appearance preference only. No native bridge, settings or AWS calls.
(function () {
  "use strict";
  const root = document.documentElement;
  const storageKey = "ui.contrast";
  const original = 0;
  const valid = value => Number.isInteger(value) && value >= -20 && value <= 50;
  const savedValue = text => text !== null && /^-?\d+$/.test(text) && valid(Number(text)) ? Number(text) : original;
  let value = original;
  let storageAvailable = true;
  try { value = savedValue(localStorage.getItem(storageKey)); }
  catch (_) { storageAvailable = false; }

  // Different surfaces can have opposite lightness within one theme (for
  // example, a dark rail beside Paper's light workspace). Keep each pairing.
  const adjustments = [
    ["--text-primary", "--bg-elev", .25],
    ["--text-secondary", "--bg-elev", .65],
    ["--text-muted", "--bg-elev", 1],
    ["--border", "--bg-elev", .7],
    ["--border-strong", "--bg-elev", .65],
    ["--row-rule", "--bg-elev", .35],
    ["--style-rail-muted", "--style-rail", 1],
    ["--style-rail-border", "--style-rail", .7],
    ["--style-rail-active-ink", "--style-rail-active", .25],
    ["--style-hero-ink", "--style-hero", .25],
    ["--style-hero-muted", "--style-hero", .8],
    ["--style-hero-heading", "--style-hero", .6],
    ["--style-route-border", "--style-route", .7],
  ];
  const owned = new Map();
  const label = () => value === 0 ? "Original (0)" : value > 0 ? `Stronger (+${value})` : `Softer (${value})`;

  function contrastingEndpoint(background) {
    let channels;
    if (/^#[\da-f]{3,8}$/i.test(background)) {
      let hex = background.slice(1);
      if (hex.length === 3 || hex.length === 4) hex = [...hex].map(c => c + c).join("");
      channels = [0, 2, 4].map(index => parseInt(hex.slice(index, index + 2), 16));
    } else if (/^rgba?\(/i.test(background)) {
      channels = background.match(/[\d.]+/g)?.slice(0, 3).map(Number);
    }
    if (!channels || channels.length !== 3 || channels.some(channel => !Number.isFinite(channel))) return null;
    const linear = channels.map(channel => {
      const s = channel / 255;
      return s <= .04045 ? s / 12.92 : ((s + .055) / 1.055) ** 2.4;
    });
    return .2126 * linear[0] + .7152 * linear[1] + .0722 * linear[2] > .179 ? "black" : "white";
  }

  function restoreOriginalTokens() {
    for (const [name, previous] of owned) {
      if (previous.value) root.style.setProperty(name, previous.value, previous.priority);
      else root.style.removeProperty(name);
    }
    owned.clear();
  }

  function apply() {
    // Read the source theme afresh. Never accumulate a mix from an earlier
    // slider movement or carry Paper's colors into another appearance.
    restoreOriginalTokens();
    root.dataset.contrast = String(value);
    if (value === original) return;
    const computed = getComputedStyle(root);
    const bases = adjustments.map(([name, background, weight]) => ({
      name, weight, color: computed.getPropertyValue(name).trim(),
      background: computed.getPropertyValue(background).trim(),
    }));
    for (const item of bases) {
      if (!item.color || !item.background) continue;
      const endpoint = value > 0 ? contrastingEndpoint(item.background) : item.background;
      if (!endpoint) continue;
      // Softening is deliberately modest; stronger contrast emphasizes muted
      // labels and boundaries while retaining the chosen palette and accents.
      const amount = Math.abs(value) * item.weight * (value < 0 ? .6 : 1);
      const mixed = `color-mix(in srgb, ${item.color} ${100 - amount}%, ${endpoint})`;
      if (!CSS.supports("color", mixed)) continue;
      owned.set(item.name, { value: root.style.getPropertyValue(item.name), priority: root.style.getPropertyPriority(item.name) });
      root.style.setProperty(item.name, mixed);
    }
  }

  function syncControls() {
    const slider = document.querySelector("#appearance-contrast");
    const output = document.querySelector("#contrast-value");
    const status = document.querySelector("#contrast-status");
    if (slider) { slider.value = String(value); slider.setAttribute("aria-valuetext", label()); }
    if (output) output.textContent = label();
    if (status) status.textContent = storageAvailable
      ? value === original ? "Original contrast. Changes save on this device." : "Contrast saved on this device."
      : "Contrast applies for this session. Local storage is unavailable; this choice may not survive a restart.";
  }

  function choose(next) {
    if (!valid(next)) return;
    value = next;
    apply();
    try {
      if (value === original) localStorage.removeItem(storageKey);
      else localStorage.setItem(storageKey, String(value));
      storageAvailable = true;
    } catch (_) { storageAvailable = false; }
    syncControls();
  }

  function init() {
    document.querySelector("#appearance-contrast")?.addEventListener("input", event => choose(Number(event.target.value)));
    document.querySelector("#contrast-reset")?.addEventListener("click", () => choose(original));
    apply();
    syncControls();
    // Observe only source-palette selectors, not our style attribute or UI.
    new MutationObserver(apply).observe(root, {
      attributes: true, attributeFilter: ["data-theme", "data-design", "data-appearance"],
    });
    window.addEventListener("storage", event => {
      if (event.key !== storageKey && event.key !== null) return;
      value = savedValue(event.newValue);
      storageAvailable = true;
      apply();
      syncControls();
    });
  }
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", init, { once: true });
  else init();
})();
