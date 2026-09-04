import { expect, test } from "@playwright/test";

const stackWidget = (page) => page.locator('.widget[data-widget="cfn-stacks"]');
const stackRows = (page) => stackWidget(page).locator(
  ".cfn-stacks-body > table.events-table > tbody > tr:not(.row-detail)"
);
const visibleStackRows = (page) => stackWidget(page).locator(
  ".cfn-stacks-body > table.events-table > tbody > tr:not(.row-detail):not([hidden])"
);
const codeArtifactWidget = (page) => page.locator('.widget[data-widget="codeartifact-packages"]');
const codeArtifactRows = (page) => codeArtifactWidget(page).locator(
  ".codeartifact-packages-rows > table.events-table > tbody > tr:not(.row-detail)"
);

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".brand-tag")).toHaveText("browser mode");
});

test("starts in browser mode and renders mock widgets", async ({ page }) => {
  await expect(page).toHaveTitle("Cloud Burrito");
  await expect(page.locator(".brand-name")).toHaveText("Cloud Burrito");

  const coreStatus = page.locator("#core-status");
  await expect(coreStatus).toHaveAttribute("data-state", "offline");
  await expect(coreStatus.locator(".core-label")).toHaveText("browser mode");

  const account = page.getByRole("combobox", { name: "Default account" });
  await expect(account).toBeDisabled();
  await expect(account).toHaveValue("(browser mode — no profiles)");

  const region = page.getByRole("combobox", { name: "Default region" });
  await expect(region).toBeEnabled();
  await expect(region).toHaveValue("eu-west-1");

  await expect(page.locator("#grid-stack > .grid-stack-item")).toHaveCount(6);
  await expect(page.getByRole("heading", { name: "CloudFormation Stacks" })).toBeVisible();
  await expect(stackRows(page)).toHaveCount(4);
  await expect(stackWidget(page).locator(".table-filter-count")).toHaveText("4");
});

test("omits the unavailable global search shortcut and keeps the persistent region picker", async ({ page }) => {
  const globalSearch = page.getByRole("textbox", { name: "Global search" });
  await expect(globalSearch).toHaveCount(0);
  const filter = stackWidget(page).getByRole("textbox", { name: "Search stacks (name, status, or date)…" });
  await filter.focus();
  await page.evaluate(() => {
    window.__shortcutPrevented = null;
    document.addEventListener("keydown", event => {
      if (event.ctrlKey && event.key.toLowerCase() === "k") window.__shortcutPrevented = event.defaultPrevented;
    });
  });
  await page.keyboard.press("Control+K");
  await expect(globalSearch).toHaveCount(0);
  expect(await page.evaluate(() => window.__shortcutPrevented)).toBe(false);

  const region = page.getByRole("combobox", { name: "Default region" });
  await region.click();

  const list = page.locator("#topbar-picker-list");
  await expect(list).toBeVisible();
  await expect(list.getByRole("option")).toHaveCount(2);
  await expect(region).toHaveAttribute("aria-expanded", "true");

  await region.fill("us east");
  await expect(list.getByRole("option")).toHaveCount(1);
  await expect(list.getByRole("option")).toHaveText("us-east-1");
  await expect(page.locator("#topbar-picker-status")).toHaveText("1 matching option");

  await region.press("Enter");
  await expect(region).toHaveValue("us-east-1");
  await expect(region).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator("#region-select")).toHaveValue("us-east-1");
  await expect(list).toBeHidden();
  await expect(stackWidget(page).locator(".widget-context")).toHaveText(
    "Default · (none) · (none) · us-east-1"
  );

  await expect.poll(() => page.evaluate(() => {
    const raw = localStorage.getItem("acc.last.v1");
    return raw ? JSON.parse(raw).region : null;
  })).toBe("us-east-1");

  await page.reload();
  await expect(page.getByRole("combobox", { name: "Default region" })).toHaveValue("us-east-1");
  await expect(stackWidget(page).locator(".widget-context")).toHaveText(
    "Default · (none) · (none) · us-east-1"
  );
});

test("beta exposes working widget controls and honest log-count naming", async ({ page }) => {
  await expect(page.locator("#global-search, #ai-prompt, #ai-generate, #ai-output")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Pause", exact: true })).toHaveCount(0);
  await expect(page.locator(".dot-live")).toHaveCount(0);
  await expect(page.locator('[data-widget="errors-by-stack"] .widget-title')).toHaveText("Log Error Counts");
  await expect(page.locator(".ro-badge")).toHaveText("RESOURCE READS");

  const filter = stackWidget(page).getByRole("textbox", { name: "Search stacks (name, status, or date)…" });
  await filter.fill("inventory");
  await expect(visibleStackRows(page)).toHaveCount(1);
  await expect(visibleStackRows(page)).toContainText("inventory");
  await filter.clear();

  await page.locator("#add-widget-btn").click();
  const panel = page.locator("#side-panel");
  await expect(panel).toHaveAttribute("aria-hidden", "false");
  await expect(panel.getByRole("heading", { name: "Add a widget" })).toBeVisible();
  await expect(panel.getByRole("heading", { name: "Describe with AI" })).toHaveCount(0);
  await expect(panel.getByRole("button", { name: "Generate", exact: true })).toHaveCount(0);
  const catalogItem = panel.locator(".prebuilt-item").filter({ has: page.locator(".prebuilt-name", { hasText: /^Log Error Counts$/ }) });
  await expect(catalogItem).toHaveCount(1);
  await catalogItem.getByRole("button", { name: "Add", exact: true }).click();
  await expect(panel).toHaveAttribute("aria-hidden", "true");
  await expect(page.locator('[data-widget="errors-by-stack"]')).toHaveCount(2);
  await expect(page.locator('[data-widget="errors-by-stack"] .widget-title')).toHaveText(["Log Error Counts", "Log Error Counts"]);
  await expect(page.locator("#grid-stack > .grid-stack-item")).toHaveCount(7);
  await expect(page.locator(".dot-live, button[title='Pause']")).toHaveCount(0);
});

test("clipboard denial stays truthful and a synthetic retry reports success", async ({ page }) => {
  await page.clock.install();
  await page.evaluate(() => {
    const fixture = window.__clipboardFixture = { allowed: false, writes: [], fallbackCalls: [] };
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
      writeText: async text => {
        fixture.writes.push(text);
        if (!fixture.allowed) throw new DOMException("Synthetic clipboard denial", "NotAllowedError");
      },
    } });
    document.execCommand = command => { fixture.fallbackCalls.push(command); return false; };
  });
  const version = "1.2.1.260701.093637+9c0581c";
  const row = codeArtifactRows(page).filter({ hasText: "example-config-library" });
  const copy = row.getByRole("button", { name: `Copy latest version ${version}` });
  const original = await copy.textContent();
  await copy.click();
  await expect(copy).toHaveText("Copy failed");
  await expect(copy).toHaveAttribute("data-copy-state", "failed");
  await expect(codeArtifactWidget(page).locator("tr.row-detail")).toHaveCount(0);
  expect(await page.evaluate(() => window.__clipboardFixture.writes)).toEqual([version]);
  expect(await page.evaluate(() => window.__clipboardFixture.fallbackCalls)).toEqual(["copy"]);
  await page.clock.fastForward(2_000);
  await expect(copy).toHaveText(original);

  await page.evaluate(() => { window.__clipboardFixture.allowed = true; });
  await copy.click();
  await expect(copy).toHaveText("✓");
  await expect(copy).toHaveAttribute("data-copy-state", "copied");
  expect(await page.evaluate(() => window.__clipboardFixture.writes)).toEqual([version, version]);
  expect(await page.evaluate(() => window.__clipboardFixture.fallbackCalls)).toEqual(["copy"]);
  await page.clock.fastForward(2_000);
  await expect(copy).toHaveText(original);
});

test("filters and expands CloudFormation mock stacks", async ({ page }) => {
  const widget = stackWidget(page);
  const rows = stackRows(page);
  const count = widget.locator(".table-filter-count");
  const table = widget.locator(".cfn-stacks-body > table.events-table");
  const filter = widget.getByRole("textbox", {
    name: "Search stacks (name, status, or date)…"
  });

  await expect(rows).toHaveCount(4);
  await rows.filter({ hasText: "uc-payment-service-prod" }).click();

  const detail = widget.locator(
    ".cfn-stacks-body > table.events-table > tbody > tr.row-detail"
  );
  await expect(detail).toBeVisible();
  await expect(rows.filter({ hasText: "uc-payment-service-prod" })).toHaveAttribute("aria-expanded", "true");
  await expect(rows.filter({ hasText: "uc-payment-service-prod" })).toHaveClass(/expanded/);
  await expect(detail.getByRole("tablist", { name: "Stack detail view" })).toBeVisible();
  const resourceTab = detail.getByRole("tab", { name: "Resources 4" });
  const eventTab = detail.getByRole("tab", { name: "Events 4" });
  await expect(resourceTab).toHaveAttribute("aria-selected", "true");
  await expect(eventTab).toHaveAttribute("aria-selected", "false");
  await expect(detail).toContainText("PaymentLambdaV2");
  await expect(detail.locator(
    '.stack-resource-logical[title="PaymentLambdaExecutionRoleForProductionWorkloads00000000"] wbr'
  )).toHaveCount(7);
  await eventTab.click();
  await expect(eventTab).toHaveAttribute("aria-selected", "true");
  await expect(detail.getByRole("tabpanel", { name: "Resources 4" })).toBeHidden();
  const eventPanel = detail.getByRole("tabpanel", { name: "Events 4" });
  await expect(eventPanel).toBeVisible();
  await expect(eventPanel).toContainText("UPDATE FAILED");
  await expect(eventPanel).toContainText("rolling back to the previous configuration");
  await resourceTab.click();
  await expect(resourceTab).toHaveAttribute("aria-selected", "true");

  await filter.fill("rollback");
  await expect(visibleStackRows(page)).toHaveCount(1);
  await expect(count).toHaveText("1 / 4");

  await filter.fill("^uc-(order|inventory)");
  await expect(visibleStackRows(page)).toHaveCount(2);
  await expect(detail).toBeHidden();
  await expect(count).toHaveText("2 / 4");

  await filter.fill("[");
  await expect(filter).toHaveClass(/table-filter-bad/);
  await expect(count).toHaveText("0 / 4");
  await expect(widget.locator(".table-filter-empty")).toHaveText("No matching stacks.");
  await expect(table).toBeHidden();

  await filter.clear();
  await expect(filter).not.toHaveClass(/table-filter-bad/);
  await expect(visibleStackRows(page)).toHaveCount(4);
  await expect(count).toHaveText("4");
  await expect(table).toBeVisible();
});

test("copies and expands CodeArtifact package versions", async ({ page, context }) => {
  const widget = codeArtifactWidget(page);
  const form = widget.locator(".codeartifact-packages-config");
  const labels = form.locator(":scope > label");

  await expect(widget.locator(".widget-sub")).toHaveText("Latest package versions by prefix");
  await expect(widget.getByRole("button", { name: "Load packages" })).toHaveCount(0);
  await expect(labels).toHaveCount(4);

  const fieldLayout = await labels.evaluateAll((nodes) => nodes.map((label) => {
    const key = label.querySelector("span").getBoundingClientRect();
    const input = label.querySelector("input").getBoundingClientRect();
    return { keyTop: key.top, keyBottom: key.bottom, inputTop: input.top };
  }));
  expect(Math.max(...fieldLayout.map(field => field.keyTop))
    - Math.min(...fieldLayout.map(field => field.keyTop))).toBeLessThan(2);
  expect(Math.max(...fieldLayout.map(field => field.inputTop))
    - Math.min(...fieldLayout.map(field => field.inputTop))).toBeLessThan(2);
  fieldLayout.forEach(field => expect(field.inputTop).toBeGreaterThan(field.keyBottom));

  const row = codeArtifactRows(page).filter({ hasText: "example-config-library" });
  const latestVersion = "1.2.1.260701.093637+9c0581c";
  const oldestVersion = "1.1.0.260217.143437+14d2544";
  const copyLatest = row.getByRole("button", { name: `Copy latest version ${latestVersion}` });
  await context.grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });
  await copyLatest.click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(latestVersion);
  await expect(widget.locator("tr.row-detail")).toHaveCount(0);

  await row.locator("td").first().click();
  await expect(row).toHaveAttribute("aria-expanded", "true");
  const detail = widget.locator("tr.row-detail");
  await expect(detail).toBeVisible();
  await expect(detail.locator(".codeartifact-version-item")).toHaveCount(10);
  await expect(detail.locator("time.codeartifact-version-published")).toHaveCount(10);
  await expect(detail).toContainText(latestVersion);
  await expect(detail).toContainText(oldestVersion);
  await expect(detail).toContainText("Published 01 Jul 2026");

  const fullVersionStyle = await detail.locator(".codeartifact-version-value").first().evaluate(
    (node) => {
      const style = getComputedStyle(node);
      return {
        overflowWrap: style.overflowWrap,
        textOverflow: style.textOverflow,
        whiteSpace: style.whiteSpace,
      };
    }
  );
  expect(fullVersionStyle).toEqual({
    overflowWrap: "anywhere",
    textOverflow: "clip",
    whiteSpace: "normal",
  });
  expect(await detail.locator(".codeartifact-version-history").evaluate(
    (node) => node.scrollWidth <= node.clientWidth + 1
  )).toBe(true);

  const versionList = detail.locator(".codeartifact-version-list");
  expect(await versionList.evaluate(
    (node) => getComputedStyle(node).gridTemplateColumns.split(" ").length
  )).toBe(1);
  await widget.locator(".fs-btn").click();
  expect(await versionList.evaluate(
    (node) => getComputedStyle(node).gridTemplateColumns.split(" ").length
  )).toBe(2);
  await widget.locator(".fs-btn").click();

  await row.locator("td").first().click();
  await expect(row).toHaveAttribute("aria-expanded", "false");
  await expect(detail).toHaveCount(0);

  await row.focus();
  await row.press("Enter");
  await expect(row).toHaveAttribute("aria-expanded", "true");
  await row.press("Space");
  await expect(row).toHaveAttribute("aria-expanded", "false");
});

test("uses CodeArtifact refresh as the first-load and reload action", async ({ page }) => {
  await page.addInitScript(() => {
    window.__codeArtifactInvocations = [];
    window.__codeArtifactHistoryInvocations = [];
    let active = null;
    const identity = {
      profile: "demo-profile", account_id: "acct-fixture", region: "eu-west-1",
    };
    const settingsDefaults = { aws_config_path: "~/.aws/config", sso_session_name: "", default_profile: "", default_region: "eu-west-1", theme: "dark" };
    let settings = { ...settingsDefaults, default_profile: identity.profile, default_region: identity.region };
    const settingsResponse = status => ({ ...settings,
      _settings: { defaults: { ...settingsDefaults }, allowed_regions: ["eu-west-1", "us-east-1"], field_errors: {} },
      _storage: { store: "settings", status } });
    const withContext = (result, payload, verified = !!active) => ({
      ...result,
      _request: {
        id: payload?.params?.request_id ?? null,
        context_id: verified ? "synthetic-context" : null,
        settings_revision: verified ? "1" : null,
        provider_revision: verified ? "1" : null,
        profile: verified ? active.profile : null,
        account_id: verified ? active.account_id : null,
        region: verified ? active.region : null,
      },
    });
    window.__TAURI__ = {
      core: {
        invoke: async (command, payload) => {
          if (command === "request_cancel") {
            const params = payload?.params || {}; if (typeof params.request_id !== "string" || !/^[A-Za-z0-9_-]{1,80}$/.test(params.request_id)) throw new Error("Invalid synthetic cancellation ID"); return { ok: true, cancelled_locally: true, cleanup_confirmed: false };
          }
          if (command === "widget_fetch") {
            if (payload?.params?.widget === "codeartifact-packages") {
              window.__codeArtifactInvocations.push(payload);
              return withContext({
                render: "table",
                columns: ["package", "latest_version", "last_published"],
                rows: [{
                  package: "demo-package",
                  latest_version: "3.2.1.260715.083000+abc1234",
                  last_published: "2026-07-15T08:30:00Z",
                  versions: [
                    "3.2.1.260715.083000+abc1234",
                    "3.2.0.260701.101500+def5678",
                    "3.1.0.260615.074500+987fedc",
                  ],
                }],
              }, payload);
            }
            if (payload?.params?.widget === "codeartifact-package-version-history") {
              window.__codeArtifactHistoryInvocations.push(payload);
              return withContext({
                render: "codeartifact_version_history",
                package: "demo-package",
                versions: [
                  { version: "3.2.1.260715.083000+abc1234", published: "2026-07-15T08:30:00Z" },
                  { version: "3.2.0.260701.101500+def5678", published: "2026-07-01T10:15:00Z" },
                  { version: "3.1.0.260615.074500+987fedc", published: "2026-06-15T07:45:00Z" },
                ],
              }, payload);
            }
            return withContext({ render: "raw_json", data: {} }, payload);
          }
          if (command === "audit_history" && payload?.params?.action === "status") return { ok: true, mode: "preserve", location: "/synthetic/audit.log", active_bytes: 0, total_bytes: 0, known_files: 0, preserve_required: false, expiry: "Preserved history never expires automatically." };
          if (command === "ping") return { version: "test" };
          if (command === "cli_availability") return { ok: true, status: "available", available: true, version_verified: false };
          if (command === "settings_get") {
            return settingsResponse("loaded");
          }
          if (command === "settings_set") {
            settings = Object.fromEntries(Object.entries(settingsDefaults).map(([name, fallback]) =>
              [name, payload?.params?.[name]?.trim() || fallback]));
            return settingsResponse("saved");
          }
          if (command === "dashboard_get") return { tiles: [], _storage: { status: "missing" } };
          if (command === "aws_list_profiles") return {
            profiles: [{ name: identity.profile, account_id: identity.account_id, region: identity.region,
              eligibility: "supported_sso", eligibility_reason: "Synthetic supported SSO profile" }],
            config_path: "/synthetic/aws/config", file_exists: true, error: null, discovery_state: "ready",
          };
          if (command === "aws_set_account") {
            active = { profile: payload.params.profile, account_id: payload.params.account_id, region: payload.params.region };
            return withContext({ ok: true, ...active }, payload);
          }
          if (command === "aws_list_pipelines") return withContext({ ok: true, pipelines: [] }, payload);
          if (command === "aws_auth_status") {
            return withContext({
              has_context: !!active, logged_in: !!active, needs_sso_login: false,
              connection_state: active ? "verified" : "unselected", ...active,
            }, payload);
          }
          return {};
        },
      },
    };
  });
  await page.reload();
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "online");

  const widget = codeArtifactWidget(page);
  const domain = widget.locator(".codeartifact-domain");
  const repository = widget.locator(".codeartifact-repository");
  const prefix = widget.locator(".codeartifact-prefix");
  const maxPackages = widget.locator(".codeartifact-max");
  const refresh = widget.getByRole("button", { name: "Load or refresh packages" });

  await expect.poll(() => page.evaluate(() => window.__codeArtifactInvocations.length)).toBe(0);
  await domain.fill("demo-domain");
  await repository.fill("demo_repo");
  await prefix.fill("demo");
  await maxPackages.fill("25");
  await refresh.click();

  await expect(widget.locator(".codeartifact-packages-rows")).toBeVisible();
  await expect(widget.locator(".codeartifact-packages-rows")).toContainText("demo-package");
  await expect.poll(() => page.evaluate(() => window.__codeArtifactInvocations.length)).toBe(1);
  await expect.poll(() => page.evaluate(() => (
    window.__codeArtifactHistoryInvocations.length
  ))).toBe(0);
  expect(await page.evaluate(() => window.__codeArtifactInvocations[0])).toEqual({
    params: {
      widget: "codeartifact-packages",
      request_id: expect.stringMatching(/^ui-[a-z0-9]+-[a-z0-9]+-\d+$/),
      inputs: {
        domain: "demo-domain",
        repository: "demo_repo",
        package_prefix: "demo",
        mode: "list",
        max_packages: 25,
      },
      context: { mode: "inherit", profile: null, account_id: null, region: null },
    },
  });

  const row = codeArtifactRows(page).filter({ hasText: "demo-package" });
  await row.locator("td").first().click();
  const detail = widget.locator("tr.row-detail");
  await expect(detail.locator("time.codeartifact-version-published")).toHaveCount(3);
  await expect(detail).toContainText("3.2.0.260701.101500+def5678");
  await expect.poll(() => page.evaluate(() => (
    window.__codeArtifactHistoryInvocations.length
  ))).toBe(1);
  expect(await page.evaluate(() => window.__codeArtifactHistoryInvocations[0])).toEqual({
    params: {
      widget: "codeartifact-package-version-history",
      request_id: expect.stringMatching(/^ui-[a-z0-9]+-[a-z0-9]+-\d+$/),
      reuse_result: true,
      inputs: {
        domain: "demo-domain",
        repository: "demo_repo",
        domain_owner: "",
        package: "demo-package",
        versions: [
          { version: "3.2.1.260715.083000+abc1234", published: "2026-07-15T08:30:00Z" },
          { version: "3.2.0.260701.101500+def5678", published: "" },
          { version: "3.1.0.260615.074500+987fedc", published: "" },
        ],
      },
      context: { mode: "inherit", profile: null, account_id: null, region: null },
    },
  });
  await row.locator("td").first().click();
  await row.locator("td").first().click();
  await expect(detail.locator("time.codeartifact-version-published")).toHaveCount(3);
  await expect.poll(() => page.evaluate(() => (
    window.__codeArtifactHistoryInvocations.length
  ))).toBe(1);

  await prefix.fill("updated");
  await refresh.click();
  await expect.poll(() => page.evaluate(() => window.__codeArtifactInvocations.length)).toBe(2);
  expect(await page.evaluate(() => (
    window.__codeArtifactInvocations[1].params.inputs.package_prefix
  ))).toBe("updated");

  await domain.fill("");
  await refresh.click();
  await expect(widget.locator(".codeartifact-packages-error")).toHaveText(
    "Domain, repository, and package prefix are required."
  );
  await expect.poll(() => page.evaluate(() => window.__codeArtifactInvocations.length)).toBe(2);
});

test("persists widget configuration across browser reloads", async ({ page }) => {
  const tile = page.locator('.grid-stack-item[gs-id="cfn-stacks"]');
  await tile.locator(".cfg-btn").click();

  const panel = page.locator("#widget-config-panel");
  await expect(panel).toHaveAttribute("aria-hidden", "false");
  await expect(page.locator("#cfg-title")).toHaveText("Configure CloudFormation Stacks");

  await panel.locator('.color-swatch[data-color="blue"]').click();
  await panel.locator("#cfg-save").click();
  await expect(panel).toHaveAttribute("aria-hidden", "true");
  await expect(tile.locator(".widget")).toHaveAttribute("data-header-color", "blue");

  await expect.poll(() => page.evaluate(() => {
    const raw = localStorage.getItem("acc.layout.v1");
    if (!raw) return null;
    const layout = JSON.parse(raw);
    return layout.find((entry) => entry.id === "cfn-stacks")?.config?.header_color ?? null;
  })).toBe("blue");

  await page.reload();
  await expect(page.locator('.grid-stack-item[gs-id="cfn-stacks"] .widget')).toHaveAttribute(
    "data-header-color",
    "blue"
  );
});
