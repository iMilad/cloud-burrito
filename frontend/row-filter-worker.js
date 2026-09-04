// Same-origin, network-free regex worker. The owner terminates this worker if
// one evaluation exceeds its deadline; a blocked regex cannot trap the UI.
let haystacks = [];
self.onmessage = event => {
  const message = event.data;
  if (message?.type === "init") {
    haystacks = Array.isArray(message.values) ? message.values.map(String) : [];
    self.postMessage({ type: "ready" });
    return;
  }
  if (message?.type === "patch") {
    for (const [index, text] of message.entries || []) {
      if (Number.isSafeInteger(index) && index >= 0 && index < haystacks.length) haystacks[index] = String(text);
    }
    return;
  }
  if (message?.type !== "match" || !Number.isSafeInteger(message.id)) return;
  const query = String(message.query || "");
  if (query.length > 512) { self.postMessage({ id: message.id, error: "Filter is limited to 512 characters. Previous matches are unchanged." }); return; }
  let matches, regex, valid = true;
  try {
    regex = new RegExp(query, "i");
  } catch (_) {
    valid = false;
  }
  if (valid) {
    try { matches = haystacks.flatMap((value, index) => regex.test(value) ? [index] : []); }
    catch (_) { self.postMessage({ id: message.id, error: "Filter evaluation failed. Previous matches are unchanged." }); return; }
  } else {
    const literal = query.toLowerCase();
    matches = haystacks.flatMap((value, index) => value.toLowerCase().includes(literal) ? [index] : []);
  }
  self.postMessage({ id: message.id, valid, matches });
};
