import { expect, test } from "@playwright/test";

// Production UI with an explicit, synthetic native bridge. No AWS, executable,
// credential source, remote request, or native webview is involved.
const failures = new WeakMap();
const widget = (page, name) => page.locator(`.widget[data-widget="${name}"]`);
const focusables = panel => panel.locator('button:visible:not([disabled]):not([tabindex="-1"]), input:visible:not([disabled]):not([tabindex="-1"]), select:visible:not([disabled]):not([tabindex="-1"]), textarea:visible:not([disabled]):not([tabindex="-1"]), summary:visible:not([tabindex="-1"]), a[href]:visible:not([tabindex="-1"]), [tabindex="0"]:visible');

async function boot(page, { theme = "dark", compact = false } = {}) {
  const errors = [];
  failures.set(page, errors);
  page.on("pageerror", error => errors.push(error.message));
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    errors.push("Unexpected remote browser request");
    return route.abort("blockedbyclient");
  });
  await page.addInitScript(({ theme, compact }) => {
    const profiles = ["a", "b"].map(suffix => ({ profile: `synthetic-${suffix}`, account_id: `acct-${suffix}-fixture`, region: "eu-west-1" }));
    const defaults = { aws_config_path: "/synthetic/config", sso_session_name: "", default_profile: "", default_region: "eu-west-1", theme: "dark" };
    let settings = { ...defaults, default_profile: profiles[0].profile, theme };
    let active = null;
    const fixture = window.__accessibility = { calls: [], unexpected: [], missing: false, failStacks: false };
    const tiles = (compact ? ["cfn-stacks"] : ["cfn-stacks", "pipeline-runs", "aws-cli", "log-tail"]).map((name, index) => ({
      id: name, widget: name, x: index % 2 ? 6 : 0, y: Math.floor(index / 2) * 6, w: compact ? 12 : 6, h: 6,
    }));
    const settingsResponse = status => ({ ...settings, _storage: { store: "settings", status },
      _settings: { defaults, allowed_regions: ["eu-west-1", "us-east-1"], field_errors: {} } });
    const attach = (result, params) => ({ ...result, _request: {
      id: params.request_id || null, context_id: active ? `synthetic-context-${active.profile}-${active.region}` : null,
      provider_revision: active ? "synthetic-provider" : null, settings_revision: active ? "1" : null,
      profile: active?.profile || null, account_id: active?.account_id || null, region: active?.region || null,
      audit_id: "synthetic-audit", outcome: result.ok === false ? "failed" : "succeeded",
    } });
    const stackName = "synthetic-stack";
    const result = params => {
      switch (params.widget) {
        case "cfn-stacks": return fixture.failStacks
          ? { ok: false, error_type: "AccessDenied", error: "Synthetic read denied" }
          : { render: "table", columns: ["stack", "status", "description"], rows: [
            { stack: stackName, status: "CREATE_COMPLETE", description: "synthetic-descriptive-resource-name-".repeat(9) },
            { stack: "synthetic-other", status: "CREATE_COMPLETE", description: "Synthetic second resource" },
          ] };
        case "cfn-stack-detail": return { render: "stack_detail", stack: stackName,
          resources: [{ logical_id: "SyntheticLogs", physical_id: "/synthetic/logs", type: "AWS::Logs::LogGroup", status: "CREATE_COMPLETE" }],
          events: [{ ts: "2026-01-01T00:00:00Z", logical_id: "SyntheticLogs", status: "CREATE_COMPLETE", reason: "Synthetic event" }] };
        case "pipeline-runs": return { render: "table", columns: ["execution_id", "status"], rows: [{ execution_id: "synthetic-execution", status: "Failed" }] };
        case "aws-cli": return { render: "table", columns: ["value"], rows: [{ value: "Synthetic CLI table" }] };
        case "log-tail":
          if (params.inputs.mode === "list") return { ok: true, functions: ["one", "two"].map(name => ({
            name: `synthetic-function-${name}`, runtime: "synthetic", state: "Active", log_group: `/synthetic/${name}`,
            handoffs: { logs: { status: "available", source: "lambda_logging_config", widget: "log-tail", inputs: { mode: "streams", log_group: `/synthetic/${name}` }, reason: "Synthetic configured log group" } },
          })) };
          if (params.inputs.mode === "streams") return { ok: true, streams: [{ name: "synthetic-stream" }] };
          return { render: "log_stream", events: [{ ts: 1700000000000, msg: "Synthetic log event", level: "info" }] };
        default: fixture.unexpected.push(`widget:${params.widget}`); throw new Error("Unplanned synthetic widget");
      }
    };
    window.__TAURI__ = { core: { invoke: async (command, payload) => {
      const params = payload?.params || {};
      fixture.calls.push({ command, params: structuredClone(params) });
      switch (command) {
        case "request_cancel": if (typeof params.request_id !== "string" || !/^[A-Za-z0-9_-]{1,80}$/.test(params.request_id)) throw new Error("Invalid synthetic cancellation ID"); return { ok: true, cancelled_locally: true, cleanup_confirmed: false };
        case "ping": return { ok: true, version: "synthetic" };
        case "settings_get": return settingsResponse("loaded");
        case "settings_set": settings = { ...settings, ...params }; return settingsResponse("saved");
        case "dashboard_get": return { tiles, _storage: { store: "dashboard", status: "loaded" } };
        case "dashboard_set": return { ok: true, _storage: { store: "dashboard", status: "saved" } };
        case "aws_list_profiles": return { discovery_state: fixture.missing ? "missing_config" : "ready", file_exists: !fixture.missing,
          profiles: fixture.missing ? [] : profiles.map(p => ({ name: p.profile, account_id: p.account_id, region: p.region,
            role_name: "SyntheticReadOnly", sso_session: "synthetic-session", eligibility: "supported_sso", eligibility_reason: null })) };
        case "aws_set_account": active = { profile: params.profile, account_id: params.account_id, region: params.region }; return attach({ ok: true }, params);
        case "aws_auth_status": return attach({ has_context: !!active, logged_in: !!active, connection_state: active ? "verified" : "unselected", ...active }, params);
        case "aws_list_pipelines": return attach({ ok: true, pipelines: ["one", "two"].map(name => ({ name: `synthetic-pipeline-${name}`, updated: "2026-01-01T00:00:00Z" })) }, params);
        case "cli_availability": return { ok: true, status: "available", available: true, version_verified: false };
        case "widget_fetch": return attach(result(params), params);
        case "policy_get": case "policy_set": return { raw: "statements: []", valid: true, actions: [], path: "/synthetic/policy" };
        case "audit_history": if (params.action !== "status") throw new Error("Unexpected audit history action"); return { ok: true, mode: "preserve", location: "/synthetic/audit.log", active_bytes: 0, total_bytes: 0, known_files: 0, preserve_required: false, expiry: "Preserved history never expires automatically." };
        case "audit_tail": return { entries: [] };
        case "widget_get_source": return { ok: true, yaml: "name: synthetic-widget", py: "// Synthetic source fixture" };
        default: fixture.unexpected.push(command); throw new Error("Unplanned synthetic native command");
      }
    } } };
  }, { theme, compact });
  await page.goto("/");
  await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
  await expect(widget(page, "cfn-stacks").locator("table")).toBeVisible();
}

test.afterEach(async ({ page }) => {
  expect(failures.get(page) || []).toEqual([]);
  expect(await page.evaluate(() => window.__accessibility?.unexpected || [])).toEqual([]);
});

async function openWithKeyboard(opener, panel) {
  await opener.focus();
  await opener.press("Enter");
  await expect(panel).toHaveAttribute("aria-hidden", "false");
  await expect(panel).toHaveAttribute("role", "dialog");
  await expect(panel).toHaveAttribute("aria-modal", "true");
  expect(await panel.evaluate(node => node.contains(document.activeElement))).toBe(true);
}

async function insideViewport(locator) {
  // Opening a panel exposes its controls before its slide-in transition finishes.
  // Assert the settled geometry without disabling motion or relaxing the bounds.
  await expect(async () => {
    await locator.scrollIntoViewIfNeeded();
    const box = await locator.boundingBox();
    const viewport = await locator.page().evaluate(() => ({ width: innerWidth, height: innerHeight }));
    expect(box).not.toBeNull();
    expect(box.x).toBeGreaterThanOrEqual(-1);
    expect(box.y).toBeGreaterThanOrEqual(-1);
    expect(box.x + box.width).toBeLessThanOrEqual(viewport.width + 1);
    expect(box.y + box.height).toBeLessThanOrEqual(viewport.height + 1);
  }).toPass({ timeout: 2_000, intervals: [50, 100, 250] });
}

async function noOuterOverflow(page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
}

test("all six panels exclude hidden controls and contain keyboard focus with predictable return", async ({ page }) => {
  await boot(page);
  await expect(page.locator('.side-panel[inert][aria-hidden="true"]')).toHaveCount(6);
  await page.locator("#add-widget-btn").focus();
  for (let i = 0; i < 12; i++) {
    await page.keyboard.press("Tab");
    expect(await page.evaluate(() => !!document.activeElement.closest('.side-panel[aria-hidden="true"]'))).toBe(false);
  }
  const cases = [
    [page.locator("#add-widget-btn"), page.locator("#side-panel")],
    [page.locator('[data-studio-action="appearance"]'), page.locator("#appearance-panel")],
    [page.locator("#settings-btn"), page.locator("#settings-panel")],
    [page.locator("#audit-btn"), page.locator("#audit-panel")],
    [page.locator("#connection-details"), page.locator("#identity-panel")],
    [widget(page, "cfn-stacks").locator(".cfg-btn"), page.locator("#widget-config-panel")],
  ];
  for (const [opener, panel] of cases) {
    await openWithKeyboard(opener, panel);
    const controls = focusables(panel);
    expect(await controls.first().evaluate(node => {
      const style = getComputedStyle(node);
      return style.outlineStyle !== "none" && parseFloat(style.outlineWidth) >= 2;
    })).toBe(true);
    await controls.last().focus();
    await page.keyboard.press("Tab");
    await expect(controls.first()).toBeFocused();
    await page.keyboard.press("Shift+Tab");
    await expect(controls.last()).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(panel).toHaveAttribute("inert", "");
    await expect(opener).toBeFocused();
  }
  const cfg = page.locator("#widget-config-panel");
  const opener = widget(page, "cfn-stacks").locator(".cfg-btn");
  await openWithKeyboard(opener, cfg);
  await cfg.locator("#cfg-save").focus();
  await page.keyboard.press("Space");
  await expect(cfg).toHaveAttribute("aria-hidden", "true");
  await expect(opener).toBeFocused();
});

test("rapid reopen, removed opener and fullscreen panels keep a usable focus and overlay", async ({ page }) => {
  await boot(page);
  await openWithKeyboard(page.locator("#settings-btn"), page.locator("#settings-panel"));
  await page.keyboard.press("Escape");
  await openWithKeyboard(page.locator("#add-widget-btn"), page.locator("#side-panel"));
  await page.waitForTimeout(280); // Outlast the previous close animation's timer.
  await expect(page.locator("#scrim")).toBeVisible();
  await page.keyboard.press("Escape");

  const stack = widget(page, "cfn-stacks");
  await stack.locator(".fs-btn").focus();
  await page.keyboard.press("Enter");
  await expect(stack).toHaveClass(/fullscreen/);
  await openWithKeyboard(page.locator("#settings-btn"), page.locator("#settings-panel"));
  const close = page.locator("#settings-panel-close");
  await expect.poll(() => close.evaluate(node => {
    const r = node.getBoundingClientRect();
    return node.contains(document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2));
  })).toBe(true);
  await page.keyboard.press("Escape");
  await expect(stack).toHaveClass(/fullscreen/);
  await page.keyboard.press("Escape");
  await expect(stack).not.toHaveClass(/fullscreen/);

  await openWithKeyboard(stack.locator(".cfg-btn"), page.locator("#widget-config-panel"));
  await stack.evaluate(node => node.closest(".grid-stack-item").remove());
  await page.keyboard.press("Escape");
  expect(await page.evaluate(() => document.activeElement !== document.body && document.activeElement.isConnected && !document.activeElement.closest("[inert]"))).toBe(true);
});

for (const compact of [false, true]) {
  test(`removing a fullscreen widget restores scrolling, grid controls and focus${compact ? " in an empty workspace" : " on a remaining widget"}`, async ({ page }) => {
    await boot(page, { compact });
    const stack = widget(page, "cfn-stacks");
    await stack.locator(".fs-btn").click();
    await expect(page.locator("body")).toHaveClass(/has-fullscreen-widget/);
    expect(await page.locator("#grid-stack").evaluate(node => [!!node.gridstack.opts.disableDrag, !!node.gridstack.opts.disableResize])).toEqual([true, true]);
    await stack.locator(".rm-btn").click();
    await expect(stack).toHaveCount(0);
    await expect(page.locator("body")).not.toHaveClass(/has-fullscreen-widget/);
    expect(await page.locator("body").evaluate(node => getComputedStyle(node).overflow)).not.toBe("hidden");
    expect(await page.locator("#grid-stack").evaluate(node => [!!node.gridstack.opts.disableDrag, !!node.gridstack.opts.disableResize])).toEqual([false, false]);
    if (compact) await expect(page.locator("#add-widget-btn")).toBeFocused();
    else await expect(page.locator(".grid-stack-item .rm-btn:focus")).toHaveCount(1);
    if (!compact) {
      const remaining = page.locator(".grid-stack-item").filter({ has: widget(page, "pipeline-runs") });
      await expect(remaining).not.toHaveClass(/ui-draggable-disabled|ui-resizable-disabled/);
      // A surviving widget can still enter and leave fullscreen normally.
      await widget(page, "pipeline-runs").locator(".fs-btn").click();
      await page.keyboard.press("Escape");
      await expect(page.locator("body")).not.toHaveClass(/has-fullscreen-widget/);
      await expect(widget(page, "pipeline-runs").locator(".fs-btn")).toBeFocused();
    }
  });
}

test("external fullscreen removal restores prior grid restrictions without stealing modal focus", async ({ page }) => {
  await boot(page);
  await page.locator("#grid-stack").evaluate(node => node.gridstack.enableMove(false));
  const stack = widget(page, "cfn-stacks");
  await stack.locator(".fs-btn").click();
  await openWithKeyboard(page.locator("#settings-btn"), page.locator("#settings-panel"));
  const focused = await page.evaluate(() => document.activeElement.id);
  await stack.evaluate(node => node.closest(".grid-stack-item").remove());
  await expect(page.locator("body")).not.toHaveClass(/has-fullscreen-widget/);
  expect(await page.locator("#grid-stack").evaluate(node => [!!node.gridstack.opts.disableDrag, !!node.gridstack.opts.disableResize])).toEqual([true, false]);
  expect(await page.evaluate(() => document.activeElement.id)).toBe(focused);
  await page.keyboard.press("Escape");
  await expect(page.locator("#settings-btn")).toBeFocused();
});

async function checkTabs(tabs) {
  const first = tabs.first(), last = tabs.last();
  await first.focus();
  await first.press("ArrowRight");
  await expect(last).toBeFocused();
  await expect(last).toHaveAttribute("aria-selected", "true");
  await expect(first).toHaveAttribute("tabindex", "-1");
  await last.press("Home");
  await expect(first).toBeFocused();
  await expect(first).toHaveAttribute("aria-selected", "true");
  await first.press("End");
  await expect(last).toBeFocused();
  await last.press("ArrowLeft");
  await expect(first).toBeFocused();
  for (const tab of [first, last]) {
    const id = await tab.getAttribute("aria-controls");
    expect(id).toBeTruthy();
    const panel = tab.page().locator(`[id="${id}"]`);
    await expect(panel).toHaveAttribute("role", "tabpanel");
    await expect(panel).toHaveAttribute("aria-labelledby", await tab.getAttribute("id"));
  }
}

test("pipeline, CLI and stack tabs support arrows, Home/End and explicit panel relationships", async ({ page }) => {
  await boot(page);
  await checkTabs(widget(page, "pipeline-runs").getByRole("tab"));
  await checkTabs(widget(page, "aws-cli").getByRole("tab"));
  const row = widget(page, "cfn-stacks").locator("tr.expandable").first();
  await row.focus();
  await row.press("Enter");
  const detail = widget(page, "cfn-stacks").locator(".row-detail");
  await expect(detail).toContainText("SyntheticLogs");
  await checkTabs(detail.getByRole("tab"));
});

test("topbar and pipeline comboboxes expose active options and Escape consumes only the popup", async ({ page }) => {
  await boot(page);
  const account = page.locator("#account-picker-search");
  await account.focus();
  await account.press("ArrowDown");
  let active = await account.getAttribute("aria-activedescendant");
  expect(active).toBeTruthy();
  await expect(page.locator(`[id="${active}"]`)).toHaveAttribute("aria-selected", "true");
  await account.press("Escape");
  await expect(account).toBeFocused();
  await expect(account).toHaveAttribute("aria-expanded", "false");

  const pipeline = widget(page, "pipeline-runs");
  await pipeline.locator(".fs-btn").focus();
  await page.keyboard.press("Enter");
  const input = pipeline.locator(".pipeline-name-search");
  await expect(input).toBeEnabled();
  await input.focus();
  await input.press("ArrowDown");
  active = await input.getAttribute("aria-activedescendant");
  expect(active).toBeTruthy();
  await expect(page.locator(`[id="${active}"]`)).toHaveAttribute("aria-selected", "true");
  await expect(input).toHaveAttribute("aria-controls", "pipeline-combo-list");
  await input.press("Escape");
  await expect(input).toBeFocused();
  await expect(input).toHaveAttribute("aria-expanded", "false");
  await expect(pipeline).toHaveClass(/fullscreen/);
  await input.press("ArrowDown");
  await input.press("Enter");
  await expect(pipeline).toContainText("synthetic-execution");
});

test("selection, refresh and local filters preserve focus without repeating long result announcements", async ({ page }) => {
  await boot(page);
  const lambda = widget(page, "log-tail");
  const choice = lambda.getByRole("button", { name: /synthetic-function-one/ });
  await choice.focus();
  await choice.press("Enter");
  await expect(choice).toBeFocused();
  await expect(choice).toHaveAttribute("aria-pressed", "true");
  await expect(lambda.locator(".lambda-stream-select")).toHaveAccessibleName(/stream/i);

  await openWithKeyboard(widget(page, "cfn-stacks").locator(".cfg-btn"), page.locator("#widget-config-panel"));
  const swatch = page.locator('.color-swatch[data-color="blue"]');
  await swatch.focus();
  await swatch.press("Space");
  await expect(swatch).toBeFocused();
  await expect(swatch).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("Escape");
  await openWithKeyboard(page.locator("#connection-details"), page.locator("#identity-panel"));
  const refresh = page.locator("#identity-panel").getByRole("button", { name: "Refresh", exact: true });
  await page.evaluate(() => {
    window.__connectionAnnouncements = [];
    const node = document.getElementById("connection-announcement");
    if (!node) throw new Error("Missing concise connection announcer");
    window.__connectionObserver = new MutationObserver(() => window.__connectionAnnouncements.push(node.textContent));
    window.__connectionObserver.observe(node, { childList: true, subtree: true, characterData: true });
  });
  const authCalls = await page.evaluate(() => window.__accessibility.calls.filter(call => call.command === "aws_auth_status").length);
  await refresh.focus();
  await refresh.press("Enter");
  await expect.poll(() => page.evaluate(() => window.__accessibility.calls.filter(call => call.command === "aws_auth_status").length)).toBe(authCalls + 1);
  await expect(refresh).toBeFocused();
  expect(await page.evaluate(() => window.__connectionAnnouncements)).toEqual([]);
  await expect(page.locator('.connection-copy[role="status"]')).toHaveCount(0);
  await page.keyboard.press("Escape");

  const stack = widget(page, "cfn-stacks");
  // Coverage remains readable but is not itself an endlessly recreated live region.
  await expect(stack.locator('.result-status[role="status"], .result-status[aria-live="polite"]')).toHaveCount(0);
  await page.evaluate(() => {
    window.__resultAnnouncements = [];
    const node = document.getElementById("result-announcement");
    if (!node) throw new Error("Missing concise result announcer");
    window.__announcementObserver = new MutationObserver(() => window.__resultAnnouncements.push(node.textContent));
    window.__announcementObserver.observe(node, { childList: true, subtree: true, characterData: true });
  });
  const filter = stack.getByRole("textbox", { name: "Search stacks (name, status, or date)…" });
  await filter.fill("other");
  await filter.clear();
  expect(await page.evaluate(() => window.__resultAnnouncements)).toEqual([]);
  await page.evaluate(() => { window.__accessibility.failStacks = true; });
  await stack.locator('[title="Refresh"]').focus();
  await page.keyboard.press("Enter");
  await expect(stack.locator(".result-status")).toHaveAttribute("data-state", "stale");
  await expect.poll(() => page.evaluate(() => window.__resultAnnouncements.length)).toBeGreaterThan(0);
  expect(await page.evaluate(() => window.__resultAnnouncements.every(text => text.length < 220 && !text.includes("Verified account:")))).toBe(true);
});

for (const theme of ["light", "dark"]) {
  test(`${theme} theme keeps desktop actions readable at compact sizes, zoom and doubled text`, async ({ page }) => {
    test.setTimeout(60_000);
    await boot(page, { theme, compact: true });
    const sizes = [
      { width: 1280, height: 800, zoom: 1, text: 1 },
      { width: 1024, height: 768, zoom: 1, text: 1 },
      { width: 1024, height: 768, zoom: 2, text: 1 },
      { width: 1280, height: 800, zoom: 1, text: 2 },
    ];
    for (const size of sizes) {
      await page.setViewportSize({ width: size.width, height: size.height });
      await page.evaluate(({ zoom, text }) => {
        document.documentElement.style.zoom = String(zoom);
        for (const node of document.querySelectorAll("[data-font-before]")) {
          node.style.fontSize = "";
          delete node.dataset.fontBefore;
        }
        if (text === 2) {
          const nodes = Array.from(document.querySelectorAll("body :is(button,input,select,textarea,.small,p,label,h2,td,th)"));
          const fonts = nodes.map(node => parseFloat(getComputedStyle(node).fontSize));
          nodes.forEach((node, index) => { node.dataset.fontBefore = String(fonts[index]); node.style.fontSize = `${fonts[index] * 2}px`; });
        }
      }, size);
      if (size.text === 2) {
        expect(await page.locator("#settings-save").evaluate(node => parseFloat(getComputedStyle(node).fontSize) / Number(node.dataset.fontBefore))).toBeCloseTo(2);
      }
      await noOuterOverflow(page);
      for (const selector of ["#account-picker-search", "#region-picker-search", "#settings-btn", "#add-widget-btn", "#connection-retry", "#connection-details"]) {
        await insideViewport(page.locator(selector));
      }
      const stack = widget(page, "cfn-stacks");
      expect(await stack.locator("table").evaluate(node => {
        for (let parent = node.parentElement; parent; parent = parent.parentElement) {
          if (/(auto|scroll)/.test(getComputedStyle(parent).overflowX)) return true;
          if (parent === document.body) break;
        }
        return false;
      })).toBe(true);
      await openWithKeyboard(page.locator("#settings-btn"), page.locator("#settings-panel"));
      const save = page.locator("#settings-save");
      await save.focus();
      await insideViewport(save);
      await expect(save).toBeEnabled();
      const before = await page.evaluate(() => window.__accessibility.calls.filter(call => call.command === "settings_set").length);
      await save.press("Enter");
      await expect.poll(() => page.evaluate(() => window.__accessibility.calls.filter(call => call.command === "settings_set").length)).toBe(before + 1);
      await expect(page.locator("#settings-status")).toContainText("Saved");
      await noOuterOverflow(page);
      if (size.zoom === 2) await page.screenshot({ path: `/private/tmp/cloud-burrito-p207-${theme}-zoom-settings.png` });
      await page.keyboard.press("Escape");
      await page.locator("#connection-retry").focus();
      await insideViewport(page.locator("#connection-retry"));
      await page.keyboard.press("Enter");
      await expect(page.locator("#connection-status")).toHaveAttribute("data-state", "verified");
      if (size.width === 1024 && size.zoom === 1) await page.screenshot({ path: `/private/tmp/cloud-burrito-p207-${theme}-compact.png` });
    }

    // Contrast is checked against actual computed foreground/opaque backgrounds,
    // not against obsolete theme declarations or transparent guesses.
    await page.evaluate(() => { document.documentElement.style.zoom = "1"; });
    await openWithKeyboard(page.locator("#settings-btn"), page.locator("#settings-panel"));
    const ratios = await page.evaluate(() => {
      function color(value) { const nums = value.match(/[\d.]+/g).map(Number); return nums.slice(0, 3); }
      function luminance(rgb) { const channels = rgb.map(v => v / 255).map(v => v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4); return channels.reduce((sum, v, i) => sum + v * [0.2126, 0.7152, 0.0722][i], 0); }
      return ["#settings-save", "#settings-aws-config-path"].map(selector => {
        const style = getComputedStyle(document.querySelector(selector));
        const a = luminance(color(style.color)), b = luminance(color(style.backgroundColor));
        return { selector, ratio: (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05) };
      });
    });
    for (const pair of ratios) expect(pair.ratio, pair.selector).toBeGreaterThanOrEqual(4.5);
  });
}
