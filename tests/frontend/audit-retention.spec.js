import { expect, test } from "@playwright/test";
import { FIXTURE_REVISION, FIXTURE_SEED, installSyntheticBridge } from "../performance/browser-fixture.mjs";

const failures = new WeakMap();
const entries = (start, count) => Array.from({ length: count }, (_, i) => ({ ts: 1700000000 + start + i, kind: "request", event: "succeeded", command: `synthetic-${start + i}` }));
function installAudit(options) {
  const original = window.__TAURI__.core.invoke;
  const f = window.__auditFixture = { calls: [], mode: options.mode || "preserve", effective: options.effective || options.mode || "preserve",
    legacy: options.legacy === true, oversized: options.oversized === true, pages: [], tailActive: 0, tailPeak: 0, holdTail: false,
    preserveFailure: false, preserveCalls: 0 };
  const status = () => ({ ok: true, mode: f.effective, location: "/synthetic/audit.log", active_bytes: f.oversized ? 20 * 1048576 : 1024,
    total_bytes: f.oversized ? 20 * 1048576 : 1024, known_files: 1, oversized_legacy: f.oversized, preserve_required: f.oversized,
    limits: { files: 5, bytes_per_file: 10485760, total_bytes: 52428800 },
    expiry: "The oldest of five files expires before rotation in bounded mode. Preserved history never expires automatically." });
  window.__TAURI__.core.invoke = async (command, payload) => {
    const params = payload?.params || {};
    f.calls.push({ command, params: structuredClone(params) });
    if (command === "audit_history") {
      if (params.action === "status") return status();
      if (params.action !== "preserve") throw Error("Unexpected synthetic audit history action");
      f.preserveCalls++;
      if (f.preserveFailure) return { ok: false, error_type: "AuditHistoryFailed", error: "Synthetic private diagnostic must not display" };
      f.oversized = false;
      return { ok: true, preserved_files: 1, preserved_bytes: 20 * 1048576, preserved_location: "/synthetic/audit-preserved-fixture" };
    }
    if (command === "audit_tail") {
      f.tailActive++; f.tailPeak = Math.max(f.tailPeak, f.tailActive);
      try {
        if (f.holdTail) await new Promise(resolve => { f.releaseTail = resolve; });
        return f.pages.shift() || { entries: [], cursor: params.cursor || "synthetic-empty", bytes_read: 0, skipped: 0, has_more: false, reset: false, limited: false, partial_tail: false };
      } finally { f.tailActive--; }
    }
    if (command === "settings_set" && params.audit_retention === "bounded" && f.oversized) {
      const old = await original("settings_get");
      return { ok: false, error_type: "AuditPreserveRequired", preserve_required: true, _settings: old._settings };
    }
    const result = await original(command, payload);
    if (command === "settings_get" || command === "settings_set") {
      if (command === "settings_set") { f.mode = params.audit_retention || "preserve"; f.effective = f.mode; f.legacy = false; }
      if (!f.legacy) {
        result.audit_retention = f.mode;
        result._settings.defaults.audit_retention = "preserve";
        result._audit_retention = { configured_mode: f.mode, effective_mode: f.effective, preserve_required: f.oversized };
      }
    }
    return result;
  };
}
async function boot(page, options = {}) {
  const errors = []; failures.set(page, errors);
  page.on("pageerror", () => errors.push("Production page error"));
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    errors.push("Unexpected remote request"); return route.abort("blockedbyclient");
  });
  await page.clock.install();
  const fixture = { kind: "table", rows: 1, delayMs: 0, fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED };
  await page.addInitScript({ content: `(${installSyntheticBridge.toString()})(${JSON.stringify(fixture)});(${installAudit.toString()})(${JSON.stringify(options)});` });
  await page.goto("/");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
}
const calls = (page, command) => page.evaluate(command => window.__auditFixture.calls.filter(call => call.command === command), command);
const queue = (page, pages) => page.evaluate(pages => window.__auditFixture.pages.push(...pages), pages);
const auditRows = page => page.locator("#audit-table-wrap tbody tr");
test.afterEach(async ({ page }) => { expect(failures.get(page)).toEqual([]); expect(await page.evaluate(() => window.__performanceFixture.errors)).toEqual([]); });

test("older settings keep preserve default and bounded retention changes only after explicit Save", async ({ page }) => {
  await boot(page, { legacy: true }); await page.locator("#settings-btn").click();
  const panel = page.locator("#settings-panel");
  await expect(page.locator("#settings-audit-retention")).toHaveValue("preserve");
  await expect(panel.locator("[data-audit-history-status]")).toContainText("Active retention: Preserve");
  await page.locator("#settings-audit-retention").selectOption("bounded");
  expect(await calls(page, "settings_set")).toEqual([]);
  expect(await page.evaluate(() => window.__auditFixture.preserveCalls)).toBe(0);
  await expect(panel.locator("[data-audit-history-status]")).toContainText("Active retention: Preserve");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toContainText("Saved.");
  expect((await calls(page, "settings_set"))[0].params.audit_retention).toBe("bounded");
  await expect(panel.locator("[data-audit-history-status]")).toContainText("Active retention: Bounded");
});

test("oversized history requires explicit preservation and reports its surviving copy before bounded Save", async ({ page }) => {
  await boot(page, { mode: "bounded", effective: "preserve", oversized: true });
  await page.locator("#settings-btn").click(); const panel = page.locator("#settings-panel");
  await expect(panel.locator("[data-audit-history-status]")).toContainText("preserve mode is active");
  expect(await page.evaluate(() => window.__auditFixture.preserveCalls)).toBe(0);
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toContainText("Preserve current audit history");
  await panel.getByRole("button", { name: "Preserve current history" }).click();
  await expect(panel.locator("[data-audit-preserve-status]")).toContainText("/synthetic/audit-preserved-fixture");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  await expect(panel.locator("[data-audit-history-status]")).toContainText("Active retention: Bounded");
});

test("cursor polling appends at most300 rows, resets on replacement, and retains evidence on read failure", async ({ page }) => {
  await boot(page);
  await queue(page, [{ entries: entries(0, 300), cursor: "synthetic-first", limited: true }]);
  await page.locator("#audit-btn").click(); await expect(auditRows(page)).toHaveCount(300);
  await queue(page, [{ entries: entries(300, 10), cursor: "synthetic-next", skipped: 2 }]);
  await page.clock.fastForward(2100); await expect(auditRows(page).first()).toContainText("synthetic-309");
  await expect(auditRows(page)).toHaveCount(300);
  expect((await calls(page, "audit_tail"))[1].params.cursor).toBe("synthetic-first");
  await expect(page.locator("#audit-page-status")).toContainText("2 invalid or oversized");
  await queue(page, [{ ok: false, error_type: "AuditReadFailed", error: "Synthetic private diagnostic" }]);
  await page.clock.fastForward(2100); await expect(page.locator("#audit-read-warning")).toBeVisible();
  await expect(auditRows(page).first()).toContainText("synthetic-309");
  await queue(page, [{ entries: entries(999, 1), cursor: "synthetic-reset", reset: true }]);
  await page.clock.fastForward(2100); await expect(auditRows(page)).toHaveCount(1);
  await expect(auditRows(page)).toContainText("synthetic-999");
  await expect(page.locator("#audit-page-status")).toContainText("replaced");
});

test("a held audit read does not overlap and closing the panel discards its late reply", async ({ page }) => {
  await boot(page); await page.evaluate(() => { window.__auditFixture.holdTail = true; });
  await queue(page, [{ entries: entries(0, 1), cursor: "synthetic-late" }]);
  await page.locator("#audit-btn").click();
  await expect.poll(async () => (await calls(page, "audit_tail")).length).toBe(1);
  await page.clock.fastForward(10000);
  expect((await calls(page, "audit_tail")).length).toBe(1);
  await page.locator("#audit-panel-close").click();
  await page.evaluate(() => { window.__auditFixture.holdTail = false; window.__auditFixture.releaseTail(); });
  await page.clock.fastForward(5000); await expect(auditRows(page)).toHaveCount(0);
  expect(await page.evaluate(() => window.__auditFixture.tailPeak)).toBe(1);
  expect((await calls(page, "audit_tail")).length).toBe(1);
});

test("preservation failure keeps retention unchanged and uses a fixed actionable message", async ({ page }) => {
  await boot(page); await page.locator("#settings-btn").click();
  await page.evaluate(() => { window.__auditFixture.preserveFailure = true; });
  const panel = page.locator("#settings-panel");
  await panel.getByRole("button", { name: "Preserve current history" }).click();
  await expect(panel.locator("[data-audit-preserve-status]")).toContainText("did not complete");
  await expect(panel).not.toContainText("Synthetic private diagnostic");
  expect(await calls(page, "settings_set")).toEqual([]);
  await expect(panel.locator("[data-audit-history-status]")).toContainText("Active retention: Preserve");
});
