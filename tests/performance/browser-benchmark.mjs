#!/usr/bin/env node
// Opt-in synthetic measurement of the unbundled production frontend. Nothing in
// this runner starts Tauri, reads AWS configuration, or executes an AWS command.
import { chromium } from "@playwright/test";
import { createServer } from "node:http";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname, extname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";
import { FIXTURE_REVISION, FIXTURE_SEED, installSyntheticBridge } from "./browser-fixture.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const definitions = [
  ...[0, 100, 500].map(delayMs => ({ id: `startup-six-${delayMs}`, kind: "six", rows: 50, delayMs, freshProcess: true })),
  { id: "pins-50", kind: "pins", rows: 50, delayMs: 100 },
  ...[100, 1000, 10000].map(rows => ({ id: `table-${rows}`, kind: "table", rows, delayMs: 0 })),
  ...[50, 1000].map(rows => ({ id: `packages-${rows}`, kind: "packages", rows, delayMs: 100 })),
];

function argumentsForRun() {
  const options = { warmups: 5, trials: 30, scenarios: "all", chrome: process.env.CLOUD_BURRITO_CHROME_PATH };
  for (let i = 2; i < process.argv.length; i += 2) {
    const key = process.argv[i]?.replace(/^--/, "");
    if (!["output", "chrome", "scenarios", "warmups", "trials"].includes(key) || !process.argv[i + 1]) throw new Error("Expected --output PATH --chrome PATH [--scenarios IDs] [--warmups N --trials N]");
    options[key] = process.argv[i + 1];
  }
  for (const key of ["warmups", "trials"]) {
    options[key] = Number(options[key]);
    if (!Number.isInteger(options[key]) || options[key] < (key === "trials" ? 1 : 0) || options[key] > 1000) throw new Error(`Invalid ${key}`);
  }
  if (!options.output || !options.chrome) throw new Error("Explicit --output and installed --chrome are required; this runner never downloads a browser.");
  const selected = options.scenarios === "all" ? definitions.map(s => s.id) : options.scenarios.split(",");
  if (!selected.length || selected.some(id => !definitions.some(s => s.id === id))) throw new Error("Unknown scenario ID");
  return { ...options, selected: definitions.filter(s => selected.includes(s.id)) };
}

function statistics(values) {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
  if (!sorted.length) return { count: 0, median: null, p95: null, max: null };
  const middle = Math.floor(sorted.length / 2);
  return { count: sorted.length, median: sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2,
    p95: sorted[Math.ceil(sorted.length * 0.95) - 1], max: sorted.at(-1) };
}

function summarize(trials) {
  const recorded = trials.filter(t => !t.warmup);
  const metrics = [...new Set(recorded.flatMap(t => Object.keys(t.metrics)))];
  return { recorded_trials: recorded.length, failed_trials: recorded.filter(t => !t.success).length,
    metrics: Object.fromEntries(metrics.map(key => [key, statistics(recorded.map(t => t.metrics[key]))])) };
}

async function serveFrontend() {
  const frontend = resolve(root, "frontend");
  const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".png": "image/png", ".woff2": "font/woff2" };
  const server = createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url, "http://127.0.0.1").pathname);
      const path = resolve(frontend, `.${pathname === "/" ? "/index.html" : pathname}`);
      if (!path.startsWith(frontend + sep)) { response.writeHead(403).end(); return; }
      const body = await readFile(path);
      response.writeHead(200, { "Content-Type": mime[extname(path)] || "application/octet-stream", "Cache-Control": "no-store" });
      response.end(body);
    } catch { response.writeHead(404).end(); }
  });
  await new Promise((accept, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", accept); });
  return { origin: `http://127.0.0.1:${server.address().port}`, close: () => new Promise(done => server.close(done)) };
}

const rowFor = (page, id) => page.locator(`.wl-row[data-thing-id="bench-${id}"]`);
const tableRows = page => page.locator(".wl-view-host tbody > tr[data-source-row]");
const twoFrames = page => page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))));

async function settled(page) {
  await page.waitForFunction(() => window.__performanceFixture?.active === 0);
  await twoFrames(page);
  // A second check includes follow-up work started while painting the first page.
  await page.waitForFunction(() => window.__performanceFixture?.active === 0);
  await twoFrames(page);
}

async function cdpMetrics(session) {
  const response = await session.send("Performance.getMetrics");
  return Object.fromEntries(response.metrics.map(item => [item.name, item.value]));
}

async function sampledMetrics(session) {
  let stopped = false;
  let inflight = Promise.resolve();
  const samples = [];
  const sample = () => {
    if (stopped) return;
    inflight = inflight.then(async () => {
      if (!stopped) samples.push({ sampled_at_ms: performance.now(), ...await cdpMetrics(session) });
    }).catch(() => {});
  };
  sample();
  const timer = setInterval(sample, 50);
  return async () => { clearInterval(timer); await inflight; stopped = true; samples.push({ sampled_at_ms: performance.now(), ...await cdpMetrics(session) }); return samples; };
}

async function timedInput(page, selector, value) {
  return page.evaluate(async ({ selector, value }) => {
    const input = document.querySelector(selector);
    if (!input) throw new Error("Expected production filter control");
    const started = performance.now();
    input.focus();
    input.value = value;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    const handlerFinished = performance.now();
    // The baseline is synchronous; later worker-based filters expose pending.
    // Wait for real matching completion so asynchronous filtering is not
    // incorrectly reported as a faster two-frame result.
    await new Promise((done, reject) => {
      const deadline = performance.now() + 5000;
      const check = () => {
        if (input.dataset.filterPending !== "true") return done();
        if (performance.now() >= deadline) return reject(new Error("Filter did not settle"));
        requestAnimationFrame(check);
      };
      check();
    });
    await new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done)));
    return { handler_ms: handlerFinished - started, painted_ms: performance.now() - started,
      invalid: input.classList.contains("table-filter-bad"), count: input.parentElement.querySelector(".table-filter-count")?.textContent || null,
      focus_retained: document.activeElement === input };
  }, { selector, value });
}

async function refresh(page) {
  const start = await page.evaluate(() => performance.now());
  const before = await page.evaluate(() => window.__performanceFixture.widgetResponses);
  await page.keyboard.press("r");
  await page.waitForFunction(before => window.__performanceFixture.widgetResponses > before, before);
  await settled(page);
  return (await page.evaluate(() => performance.now())) - start;
}

async function tableInteractions(page) {
  await rowFor(page, "stacks").focus();
  await page.keyboard.press("Enter");
  await tableRows(page).first().waitFor();
  await settled(page);
  const selector = ".wl-view-host .table-filter";
  const selected = await timedInput(page, selector, "synthetic-stack-00000$");
  // Use an all-column regular expression that matches a row, then restore all.
  const matched = await timedInput(page, selector, "synthetic-stack-00000");
  const invalid = await timedInput(page, selector, "[");
  if (!invalid.invalid) throw new Error("Invalid regex feedback was not preserved");
  await timedInput(page, selector, "");
  // Keyboard drill-down from the first stack into its own detail, then back.
  const name = (await tableRows(page).first().locator("td").first().textContent()).trim();
  await page.evaluate(() => window.CloudBurritoWatchlist.setFocus("inspector"));
  const detailStarted = await page.evaluate(() => performance.now());
  await page.keyboard.press("Enter");
  await page.locator(".wl-crumb strong").filter({ hasText: name }).waitFor();
  await page.getByText("SyntheticLogGroup", { exact: false }).first().waitFor();
  await settled(page);
  const detailMs = (await page.evaluate(() => performance.now())) - detailStarted;
  await page.keyboard.press("Escape");
  await tableRows(page).first().waitFor();
  const refreshMs = await refresh(page);
  return { filter_handler_ms: matched.handler_ms, filter_painted_ms: matched.painted_ms,
    filter_no_match_painted_ms: selected.painted_ms, invalid_filter_painted_ms: invalid.painted_ms,
    detail_keyboard_ms: detailMs, refresh_result_ms: refreshMs,
    filter_focus_retained: matched.focus_retained ? 1 : 0 };
}

async function prepareWorkload(page, scenario) {
  await page.waitForFunction(() => window.__performanceFixture?.milestones.interactive != null);
  await settled(page);
  if (scenario.kind === "six" || scenario.kind === "packages") {
    await page.locator('.wl-row[data-thing-id="bench-packages"][data-loading="false"]').waitFor();
  }
  if (scenario.kind === "pins" && await page.locator('.wl-row[data-loading="false"]').count() !== 50) {
    throw new Error("Expected 50 completed synthetic pipeline rows");
  }
  await settled(page);
}

async function runTrial({ browser, scenario, index, warmup, origin, freshLaunchStarted = null }) {
  const errors = [];
  const trial = { index, warmup, success: false, errors, metrics: {}, counters: {}, raw: {} };
  const context = await browser.newContext({ viewport: { width: 1480, height: 920 }, serviceWorkers: "block" });
  const page = await context.newPage();
  page.setDefaultTimeout(30_000);
  page.on("pageerror", () => errors.push("Production frontend page error"));
  await context.route("**/*", route => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    errors.push("Unexpected external browser request blocked");
    return route.abort("blockedbyclient");
  });
  const session = await context.newCDPSession(page);
  await session.send("Performance.enable");
  const initial = await cdpMetrics(session);
  const stopSampling = await sampledMetrics(session);
  const outlier = scenario.id === "startup-six-100" && !warmup && index === 29;
  await page.addInitScript(installSyntheticBridge, { ...scenario, fixtureRevision: FIXTURE_REVISION, seed: FIXTURE_SEED, outlier });
  const navStarted = performance.now();
  try {
    await page.goto(origin, { waitUntil: "domcontentloaded" });
    await prepareWorkload(page, scenario);
    const fixture = await page.evaluate(() => {
      const f = window.__performanceFixture;
      return { milestones: f.milestones, errors: f.errors, commands: f.commands, widgets: f.widgets,
        logicalRequests: f.logicalRequests, peakActive: f.peakActive, completed: f.completed,
        widgetResponses: f.widgetResponses, responseBytes: f.responseBytes, responses: f.responses, now: performance.now() };
    });
    errors.push(...fixture.errors);
    const firstResponse = fixture.responses.find(r => r.command === "widget_fetch");
    trial.metrics = {
      navigation_to_interactive_ms: fixture.milestones.interactive ?? null,
      fresh_browser_launch_to_interactive_ms: scenario.freshProcess && fixture.milestones.interactive != null
        ? navStarted - freshLaunchStarted + fixture.milestones.interactive : null,
      connect_to_verified_ms: (fixture.milestones.verified ?? fixture.milestones.identityResponse) - fixture.milestones.connectStarted,
      navigation_to_first_useful_ms: fixture.milestones.firstUseful ?? null,
      first_response_to_useful_ms: firstResponse && fixture.milestones.firstUseful ? Math.max(0, fixture.milestones.firstUseful - firstResponse.response_ready_ms) : null,
      navigation_to_complete_workload_ms: fixture.now,
      ...scenario.kind === "table" ? await tableInteractions(page) : {},
    };
    if (scenario.kind === "packages") {
      await rowFor(page, "packages").focus();
      await page.keyboard.press("Enter");
      const row = page.locator(".wl-view-host tr.expandable").first();
      await row.waitFor();
      await settled(page);
      const historyStart = await page.evaluate(() => performance.now());
      await row.focus(); await row.press("Enter"); await settled(page);
      trial.metrics.lazy_history_ms = (await page.evaluate(() => performance.now())) - historyStart;
      await row.press("Enter"); await row.press("Enter"); await settled(page);
    }
    const finalFixture = await page.evaluate(() => {
      const f = window.__performanceFixture;
      return { commands: f.commands, widgets: f.widgets, logicalRequests: f.logicalRequests, peakActive: f.peakActive,
        completed: f.completed, widgetResponses: f.widgetResponses, responseBytes: f.responseBytes, responses: f.responses, errors: f.errors };
    });
    errors.push(...finalFixture.errors.filter(error => !errors.includes(error)));
    trial.counters = { synthetic_ipc_calls: finalFixture.logicalRequests, synthetic_ipc_peak_active: finalFixture.peakActive,
      synthetic_ipc_responses: finalFixture.completed, synthetic_widget_responses: finalFixture.widgetResponses,
      synthetic_response_bytes: finalFixture.responseBytes, commands: finalFixture.commands, widgets: finalFixture.widgets,
      actual_aws_attempts: null, actual_aws_retries: null, actual_aws_pages: null, backend_queue: null,
      backend_cancellations: null };
    trial.raw = { milestones: fixture.milestones, responses: finalFixture.responses, outlier,
      rendered_rows: await page.locator(".wl-view-host tbody > tr:not(.row-detail)").count(),
      rendered_things: await page.locator(".wl-row").count() };
    const beforeIdle = await cdpMetrics(session);
    await page.waitForTimeout(200);
    const afterIdle = await cdpMetrics(session);
    trial.metrics.browser_idle_task_cpu_ms_per_200ms = (afterIdle.TaskDuration - beforeIdle.TaskDuration) * 1000;
    trial.success = errors.length === 0;
  } catch (error) {
    errors.push(error?.name === "TimeoutError" ? "Scenario timed out waiting for expected production UI state" : String(error?.message || "Scenario failed").split("\n")[0].slice(0, 180));
  } finally {
    const samples = await stopSampling();
    const final = samples.at(-1);
    trial.metrics.wall_ms = performance.now() - navStarted;
    trial.metrics.browser_js_heap_peak_sampled_bytes = Math.max(...samples.map(s => s.JSHeapUsedSize));
    trial.metrics.browser_js_heap_settled_bytes = final.JSHeapUsedSize;
    trial.metrics.browser_task_cpu_ms = (final.TaskDuration - initial.TaskDuration) * 1000;
    trial.raw.browser_samples = samples.map(s => ({ elapsed_ms: s.sampled_at_ms - navStarted,
      js_heap_used_bytes: s.JSHeapUsedSize, dom_nodes: s.Nodes, documents: s.Documents,
      task_cpu_ms: (s.TaskDuration - initial.TaskDuration) * 1000 }));
    await context.close();
  }
  return trial;
}

async function main() {
  const options = argumentsForRun();
  const server = await serveFrontend();
  const launch = () => chromium.launch({ executablePath: options.chrome, headless: true, args: ["--disable-background-networking"] });
  const report = { schema_version: 1, fixture: { revision: FIXTURE_REVISION, seed: FIXTURE_SEED },
    parameters: { warmups: options.warmups, trials: options.trials, viewport: { width: 1480, height: 920 }, one_worker: true,
      sampling_interval_ms: 50, idle_sample_ms: 200, filesystem_cache: "not flushed; host cache may be warm",
      build_mode: "unbundled production frontend source, headless installed Chrome, synthetic IPC" },
    environment: { browser_version: null, node: process.version, platform: process.platform, architecture: process.arch },
    limitations: ["No native Tauri launch, credentials, AWS request, or CLI process.",
      "CDP measures browser JavaScript heap and task CPU, not process-tree RSS or native webview idle CPU. Sampling is not a proven peak.",
      "Startup uses a fresh browser process per trial; other workloads use a new isolated context in a reused browser. Machine caches are not flushed.",
      "Synthetic input dispatch measures production event handlers and two animation frames, not a hardware input device.",
      "The six-thing workload loads four things on open; the error-count and CLI things wait for an explicit run, as in the app.",
      "Failures remain in raw trials and failure counts. Missing timing values are not converted into zero or passed gates.",
      "A single five-second outlier is injected into recorded trial index 29 of startup-six-100; a shorter smoke run omits it.",
      "Browser fake-IPC counters cannot establish backend attempt, retry, service pagination, queue, or cancellation budgets." ],
    scenarios: [] };
  const save = async () => { await mkdir(dirname(resolve(options.output)), { recursive: true }); await writeFile(options.output, JSON.stringify(report, (_key, value) => typeof value === "number" && !Number.isInteger(value) ? Number(value.toFixed(6)) : value, 2) + "\n"); };
  let sharedBrowser;
  try {
    for (const definition of options.selected) {
      const scenario = { id: definition.id, definition: { ...definition, process: definition.freshProcess ? "fresh_browser_per_trial" : "reused_browser_fresh_context_per_trial" }, trials: [] };
      report.scenarios.push(scenario);
      for (let run = -options.warmups; run < options.trials; run++) {
        const started = performance.now();
        let browser;
        if (definition.freshProcess) browser = await launch();
        else { sharedBrowser ||= await launch(); browser = sharedBrowser; }
        report.environment.browser_version ||= browser.version();
        try {
          const trial = await runTrial({ browser, scenario: definition, index: run < 0 ? run + options.warmups : run,
            warmup: run < 0, origin: server.origin, freshLaunchStarted: started });
          scenario.trials.push(trial);
          scenario.summary = summarize(scenario.trials);
          await save();
          process.stdout.write(`${definition.id} ${run < 0 ? "warmup" : "trial"} ${run < 0 ? run + options.warmups : run}: ${trial.success ? "ok" : "FAILED"}\n`);
        } finally { if (definition.freshProcess) await browser.close(); }
      }
    }
  } finally { if (sharedBrowser) await sharedBrowser.close(); await server.close(); }
  const failed = report.scenarios.some(s => s.summary.failed_trials > 0 || s.trials.some(t => !t.success));
  report.success = !failed;
  await save();
  if (failed) process.exitCode = 1;
}

await main();
