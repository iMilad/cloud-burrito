const studies = {
  current: {
    number: "01", name: "Studio Original",
    summary: "The applied identity: a generous cloud, an expressive fold, and a warm investigation workspace.",
    notes: ["The exact current production icon", "A warm hero and expressive orange", "The reference for every comparison"]
  },
  precision: {
    number: "02", name: "Precision",
    summary: "A cleaner fold, flatter surfaces, and just enough orange to point the way.",
    notes: ["A tighter geometric fold", "Flatter cards and quieter dividers", "Balanced spacing, fewer accents"]
  },
  paper: {
    number: "03", name: "Paper",
    summary: "Soft corners, olive ink, and terracotta warmth. A little more breathing room.",
    notes: ["A softer cloud and petal-like fold", "Warm paper, olive, and terracotta", "Airier spacing and rounded cards"]
  },
  night: {
    number: "04", name: "Night Shift",
    summary: "Neutral surfaces, a precise seam, and a restrained ember accent. Try it in dark mode.",
    notes: ["A compact mark with a smaller fold", "Neutral ink and chalk surfaces", "Tighter rhythm and technical labels"]
  }
};

function selectStudy(key) {
  const study = studies[key];
  if (!study) return;
  document.documentElement.dataset.variant = key;
  document.querySelectorAll("[data-select]").forEach(button => {
    button.setAttribute("aria-pressed", String(button.dataset.select === key));
  });
  document.querySelectorAll(".active-icon").forEach(img => { img.src = `assets/${key}.svg`; });
  document.querySelector("#selected-title").textContent = study.name;
  document.querySelector("#selected-number").textContent = `${study.number} / LIVE STUDY`;
  document.querySelector("#proof-number").textContent = study.number;
  document.querySelector("#selected-summary").textContent = study.summary;
  document.querySelector("#design-notes").replaceChildren(...study.notes.map(note => {
    const item = document.createElement("li");
    item.textContent = note;
    return item;
  }));
}

document.querySelectorAll("[data-select]").forEach(button => {
  button.addEventListener("click", () => selectStudy(button.dataset.select));
});
document.querySelectorAll("[data-theme-select]").forEach(button => {
  button.addEventListener("click", () => {
    const theme = button.dataset.themeSelect;
    document.documentElement.dataset.previewTheme = theme;
    document.querySelectorAll("[data-theme-select]").forEach(option => {
      option.setAttribute("aria-pressed", String(option.dataset.themeSelect === theme));
    });
  });
});
