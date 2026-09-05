import { expect, test } from "@playwright/test";

const savedPolicy = "Statement: []\n# synthetic saved policy";
const errors = new WeakMap();

// Production editor and IPC wrapper with an in-memory policy file. Deferred
// reads capture bytes at request start, making stale responses deterministic.
// No credential sources, native helpers, AWS, or remote URLs are accessed.
async function boot(page, options = {}) {
  const failures = [];
  errors.set(page, failures);
  page.on("pageerror", error => failures.push(error.message));
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    failures.push("Unexpected remote request");
    return route.abort("blockedbyclient");
  });
  await page.clock.install();
  await page.addInitScript(({ savedPolicy, options }) => {
    const defaults = { aws_config_path: "/synthetic/config", sso_session_name: "", default_profile: "",
      default_region: "eu-west-1", theme: "dark", audit_retention: "preserve" };
    const fixture = window.__policyFixture = { raw: savedPolicy, calls: [], holdGet: !!options.holdGet,
      holdSet: false, failGet: !!options.failGet, failSet: false, legacyRejection: false, malformedSave: false };
    const status = raw => ({ raw, valid: true, actions: [], path: "/synthetic/policy.yaml" });
    window.__TAURI__ = { core: { invoke: async (command, payload) => {
      const params = payload?.params || {};
      fixture.calls.push({ command, params: structuredClone(params) });
      switch (command) {
        case "ping": return { version: "synthetic" };
        case "settings_get": return { ...defaults, _settings: { defaults, allowed_regions: ["eu-west-1"], field_errors: {} } };
        case "dashboard_get": return { version: 1, tiles: [] };
        case "aws_list_profiles": return { discovery_state: "missing_config", profiles: [] };
        case "aws_auth_status": return { has_context: false, logged_in: false, connection_state: "disconnected" };
        case "audit_history": return { ok: true, mode: "preserve", location: "/synthetic/audit.log",
          active_bytes: 0, total_bytes: 0, known_files: 0, preserve_required: false };
        case "policy_get": {
          const raw = fixture.raw;
          if (fixture.holdGet) await new Promise(resolve => { fixture.releaseGet = resolve; });
          if (fixture.failGet) return { ok: false, error_type: "StorageReadFailed", raw: "", valid: false };
          return status(raw);
        }
        case "policy_set": {
          const raw = params.text;
          if (fixture.holdSet) await new Promise(resolve => { fixture.releaseSet = resolve; });
          if (fixture.failSet) return { ok: false, error_type: "StorageWriteFailed", ...status(raw) };
          if (fixture.legacyRejection) return { ...status(raw), valid: false, error: "Synthetic policy rejection" };
          if (fixture.malformedSave) return { valid: true, raw: "unrelated response" };
          fixture.raw = raw;
          return { ok: true, ...status(raw) };
        }
        default: throw new Error("Unexpected synthetic boundary invocation");
      }
    } } };
  }, { savedPolicy, options });
  await page.goto("/");
  await page.locator("#settings-btn").click();
  await expect.poll(() => page.evaluate(() => window.__policyFixture.calls.filter(call => call.command === "policy_get").length)).toBe(1);
  if (!options.holdGet && !options.failGet) await expect(page.locator("#policy-editor")).toHaveValue(savedPolicy);
}

test.afterEach(async ({ page }) => {
  expect(errors.get(page) || []).toEqual([]);
  expect(await page.evaluate(() => window.__policyFixture.calls.filter(call =>
    ["aws_set_account", "widget_fetch", "dashboard_set", "settings_set"].includes(call.command)))).toEqual([]);
});

const calls = (page, command) => page.evaluate(command => window.__policyFixture.calls.filter(call => call.command === command), command);
async function reopen(page) {
  await page.locator("#settings-panel-close").click();
  await page.clock.fastForward(250);
  await page.locator("#settings-btn").click();
}

test("a delayed save preserves a newer draft, serializes saves and survives reopening", async ({ page }) => {
  await boot(page);
  const submitted = "Statement: []\n# submitted draft";
  const newer = "Statement: []\n# newer unsaved draft";
  await page.locator("#policy-editor").fill(submitted);
  await page.evaluate(() => { window.__policyFixture.holdSet = true; });
  await page.locator("#policy-save").click();
  await expect(page.locator("#policy-save")).toBeDisabled();
  await expect(page.locator("#policy-reload")).toBeDisabled();
  await expect.poll(() => page.evaluate(() => typeof window.__policyFixture.releaseSet)).toBe("function");
  await page.locator("#policy-editor").fill(newer);
  await reopen(page);
  await expect(page.locator("#policy-editor")).toHaveValue(newer);
  expect(await calls(page, "policy_get")).toHaveLength(1);
  // Programmatic events also obey ownership guards, even bypassing disabled UI.
  await page.evaluate(() => {
    document.querySelector("#policy-save").dispatchEvent(new Event("click"));
    document.querySelector("#policy-reload").dispatchEvent(new Event("click"));
  });
  expect(await calls(page, "policy_set")).toHaveLength(1);
  expect(await calls(page, "policy_get")).toHaveLength(1);
  await page.evaluate(() => { window.__policyFixture.releaseSet(); });
  await expect(page.locator("#policy-status")).toHaveText("Saved policy is valid — 0 action(s) allowed. Current edits are not saved or validated.");
  await expect(page.locator("#policy-status")).not.toHaveClass(/policy-ok/);
  await expect(page.locator("#policy-editor")).toHaveValue(newer);
  expect(await page.evaluate(() => window.__policyFixture.raw)).toBe(submitted);
  await reopen(page);
  await expect(page.locator("#policy-editor")).toHaveValue(newer);
  expect(await calls(page, "policy_get")).toHaveLength(1);
  await page.evaluate(() => { window.__policyFixture.holdSet = false; });
  await page.locator("#policy-save").click();
  await expect(page.locator("#policy-status")).toHaveText("Saved policy is valid — 0 action(s) allowed.");
  expect(await page.evaluate(() => window.__policyFixture.raw)).toBe(newer);
  await expect(page.locator("#policy-reload")).toHaveText("Reload & validate");
});

test("a delayed initial read retains text typed while loading and never labels it validated", async ({ page }) => {
  await boot(page, { holdGet: true });
  await expect(page.locator("#policy-save")).toBeDisabled();
  const draft = "Statement: []\n# typed during load";
  await page.locator("#policy-editor").fill(draft);
  await page.evaluate(() => { window.__policyFixture.releaseGet(); });
  await expect(page.locator("#policy-save")).toBeEnabled();
  await expect(page.locator("#policy-editor")).toHaveValue(draft);
  await expect(page.locator("#policy-status")).toContainText("Current edits are not saved or validated");
  await expect(page.locator("#policy-status")).not.toHaveClass(/policy-ok/);
  expect(await page.evaluate(() => window.__policyFixture.raw)).toBe(savedPolicy);
});

test("an older reload cannot replace the policy or status after a newer save", async ({ page }) => {
  await boot(page);
  await page.evaluate(() => { window.__policyFixture.holdGet = true; });
  await page.locator("#policy-reload").click();
  await expect.poll(() => page.evaluate(() => typeof window.__policyFixture.releaseGet)).toBe("function");
  const draft = "Statement: []\n# saved while an old read is pending";
  await page.locator("#policy-editor").fill(draft);
  await page.locator("#policy-save").click();
  await expect(page.locator("#policy-status")).toHaveText("Saved policy is valid — 0 action(s) allowed.");
  await page.evaluate(() => { window.__policyFixture.releaseGet(); });
  await expect(page.locator("#policy-editor")).toHaveValue(draft);
  await expect(page.locator("#policy-status")).toHaveText("Saved policy is valid — 0 action(s) allowed.");
  expect(await page.evaluate(() => window.__policyFixture.raw)).toBe(draft);
});

test("reload explicitly discards the prior draft but retains edits made after clicking it", async ({ page }) => {
  await boot(page);
  await page.locator("#policy-editor").fill("Statement: []\n# discard this draft");
  await expect(page.locator("#policy-reload")).toHaveText("Discard edits & reload");
  await page.locator("#policy-reload").click();
  await expect(page.locator("#policy-editor")).toHaveValue(savedPolicy);
  await expect(page.locator("#policy-reload")).toHaveText("Reload & validate");
  await page.locator("#policy-editor").fill("Statement: []\n# discard earlier edits");
  await page.evaluate(() => { window.__policyFixture.holdGet = true; });
  await page.locator("#policy-reload").click();
  await expect.poll(() => page.evaluate(() => typeof window.__policyFixture.releaseGet)).toBe("function");
  const later = "Statement: []\n# keep edits made after reload";
  await page.locator("#policy-editor").fill(later);
  await page.evaluate(() => { window.__policyFixture.releaseGet(); });
  await expect(page.locator("#policy-editor")).toHaveValue(later);
  await expect(page.locator("#policy-status")).toContainText("Current edits are not saved or validated");
});

test("failed reads preserve drafts and failed saves cannot claim success before a retry", async ({ page }) => {
  await boot(page, { failGet: true });
  await expect(page.locator("#policy-status")).toContainText("Policy could not be loaded");
  await expect(page.locator("#policy-save")).toBeDisabled();
  const draft = "Statement: []\n# unsaved policy";
  await page.locator("#policy-editor").fill(draft);
  await page.locator("#policy-reload").click();
  await expect(page.locator("#policy-status")).toContainText("Current edits are preserved");
  await expect(page.locator("#policy-editor")).toHaveValue(draft);
  await page.evaluate(() => { window.__policyFixture.failGet = false; });
  await page.locator("#policy-reload").click();
  await expect(page.locator("#policy-editor")).toHaveValue(savedPolicy);
  await page.locator("#policy-editor").fill(draft);
  await page.evaluate(() => { window.__policyFixture.failSet = true; });
  await page.locator("#policy-save").click();
  await expect(page.locator("#policy-status")).toContainText("Policy save failed. Current edits are not saved.");
  await expect(page.locator("#policy-status")).toHaveClass(/policy-bad/);
  await expect(page.locator("#policy-editor")).toHaveValue(draft);
  expect(await page.evaluate(() => window.__policyFixture.raw)).toBe(savedPolicy);
  await reopen(page);
  await expect(page.locator("#policy-editor")).toHaveValue(draft);
  await page.evaluate(() => { window.__policyFixture.failSet = false; });
  await page.locator("#policy-save").click();
  await expect(page.locator("#policy-status")).toHaveText("Saved policy is valid — 0 action(s) allowed.");
  expect(await page.evaluate(() => window.__policyFixture.raw)).toBe(draft);
});

for (const rejection of ["legacyRejection", "malformedSave"]) {
  test(`${rejection} never discards or validates the current draft`, async ({ page }) => {
    await boot(page);
    const draft = "Statement: []\n# preserve rejected draft";
    await page.locator("#policy-editor").fill(draft);
    await page.evaluate(rejection => { window.__policyFixture[rejection] = true; }, rejection);
    await page.locator("#policy-save").click();
    await expect(page.locator("#policy-status")).toContainText("Policy save failed");
    await expect(page.locator("#policy-editor")).toHaveValue(draft);
    await expect(page.locator("#policy-save")).toBeEnabled();
    expect(await page.evaluate(() => window.__policyFixture.raw)).toBe(savedPolicy);
  });
}
