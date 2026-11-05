// The ":" command line: parse, complete, run as an unsaved thing in the
// inspector, and save with s. Commands never run hidden work.
(function () {
  "use strict";
  const W = window.CloudBurritoWatchlist;
  const core = window.CloudBurritoCore;
  const T = window.CloudBurritoThings;
  const C = window.CloudBurritoCommands;
  const { el } = core;
  let bar, input, list, hint, suggestions = [], active = 0;

  function env() {
    const things = W.state.things;
    const names = kind => [...new Set(things.filter(t => t.kind === kind).map(t => T.name(t)))];
    return {
      profiles: W.state.profiles, defaultProfile: W.state.settings?.default_profile || "",
      defaultRegion: W.state.settings?.default_region || "", allowedRegions: W.state.allowedRegions,
      packageRepos: things.filter(t => t.kind === "package").map(t => ({ profile: t.context.profile, domain: t.inputs.domain, repository: t.inputs.repository })),
      names: { pipeline: names("pipeline"), stack: names("stack"), package: names("package") },
    };
  }

  function renderSuggestions() {
    const text = input.value;
    suggestions = text.trim() ? C.complete(text, env()).items
      : C.HELP.map(([usage, detail]) => ({ value: null, label: usage, detail }));
    active = Math.min(active, Math.max(0, suggestions.length - 1));
    list.replaceChildren(...suggestions.map((item, n) => el("li", { role: "option", id: `wl-command-option-${n}`,
      "aria-selected": String(n === active) }, el("span", {}, item.label), el("span", { class: "wl-muted" }, item.detail || ""))));
    list.hidden = !suggestions.length;
    input.setAttribute("aria-expanded", String(!list.hidden));
    if (suggestions.length) input.setAttribute("aria-activedescendant", `wl-command-option-${active}`);
    else input.removeAttribute("aria-activedescendant");
  }

  function applySuggestion() {
    const item = suggestions[active];
    if (!item?.value) return;
    const { from, to } = C.complete(input.value, env());
    input.value = input.value.slice(0, from) + item.value + input.value.slice(to);
    active = 0;
    renderSuggestions();
  }

  function open(text = "") {
    bar.hidden = false;
    input.value = text;
    hint.textContent = "";
    active = 0;
    renderSuggestions();
    input.focus();
  }

  function close() {
    bar.hidden = true;
    list.hidden = true;
    W.setFocus(W.state.focus);
  }

  function run() {
    const result = C.parse(input.value, env());
    if (!result.ok) { hint.textContent = result.error; return; }
    close();
    if (result.type === "app") {
      if (result.name === "help") { open(""); return; }
      const handler = W.appCommands[result.name];
      if (handler) handler(); else W.notice(`:${result.name} is not available on this page.`);
      return;
    }
    const thing = { id: `unsaved-${Date.now().toString(36)}`, kind: result.kind, context: result.context,
      inputs: result.inputs, order: 0, unsaved: true };
    W.inspector.show(thing);
    W.setFocus("inspector");
    const kind = T.KINDS[thing.kind];
    W.notice(kind.cost ? "Not saved. This command can cost money. Press r to run it, or s to save it."
      : kind.autoload ? "Not saved. Press s to save it to your watchlist." : "Not saved. Press r to run it, or s to save it.");
  }

  function saveShown() {
    const shown = W.inspector.current()?.thing;
    if (!shown?.unsaved) { W.notice("Only command results can be saved with s."); return; }
    if (W.state.things.length >= 200) { W.notice("The watchlist is full (200 things). Remove one, then try again."); return; }
    const thing = T.create(shown.kind, shown.context, shown.inputs, W.state.things);
    W.state.things = [...W.state.things, thing];
    W.renderList();
    W.select(thing.id, { focus: false });
    W.loadRow(thing);
    W.inspector.show(thing);
    W.save();
    W.notice(`Saved ${T.name(thing)} to your watchlist.`);
  }

  function mount() {
    input = el("input", { id: "wl-command-input", type: "text", role: "combobox", "aria-label": "Command",
      "aria-controls": "wl-command-list", "aria-expanded": "false", "aria-autocomplete": "list", autocomplete: "off", spellcheck: "false" });
    list = el("ul", { id: "wl-command-list", class: "wl-command-list", role: "listbox", "aria-label": "Suggestions", hidden: true });
    hint = el("p", { id: "wl-command-hint", class: "wl-command-hint", role: "status" });
    bar = el("div", { id: "wl-command", class: "wl-command", hidden: true },
      el("span", { class: "wl-command-prompt", "aria-hidden": "true" }, ":"), input, hint, list);
    document.getElementById("wl-keys").before(bar);
    input.addEventListener("input", () => { hint.textContent = ""; active = 0; renderSuggestions(); });
    input.addEventListener("keydown", event => {
      if (event.key === "Enter") { event.preventDefault(); run(); }
      else if (event.key === "Escape") { event.preventDefault(); close(); }
      else if (event.key === "Tab") { event.preventDefault(); applySuggestion(); }
      else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        if (!suggestions.length) return;
        active = (active + (event.key === "ArrowDown" ? 1 : -1) + suggestions.length) % suggestions.length;
        renderSuggestions();
      }
    });
    W.actions.command = () => open("");
    W.actions.save = () => saveShown();
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mount, { once: true });
  else mount();
})();
