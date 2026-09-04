import { expect, test } from "@playwright/test";

const HOSTILE = "<img src=x onerror=window.__securityInjected=1><svg onload=window.__securityInjected=2>";
const PRIVATE_MARKER = "CB_SYNTHETIC_DIAGNOSTIC_SECRET";
const table = { render: "table", columns: ["resource"], rows: [{ resource: "synthetic resource" }] };

// Run the whole production frontend with a synthetic native boundary. A pinned
// CLI tile is a convenient host for the shared render dispatcher: these models
// never run an executable or access AWS. Every browser request is local only.
async function boot(page, response = table, options = {}) {
  const remoteRequests = [];
  const consoleMessages = [];
  page.on("console", message => consoleMessages.push(message.text()));
  await page.context().route("**/*", route => {
    if (new URL(route.request().url()).origin === "http://127.0.0.1:4173") return route.continue();
    remoteRequests.push(route.request().url());
    return route.abort("blockedbyclient");
  });
  await page.addInitScript(({ response, options }) => {
    const identity = { profile: "demo-a", account_id: "acct-a-fixture", region: "eu-west-1" };
    const command = "aws sts get-caller-identity";
    let active = false;
    const fixture = window.__securityFixture = {
      response, audit: { entries: [] }, auditWriteFailed: !!options.auditWriteFailed,
      policy: "Version: '2012-10-17'\nStatement: []", rejectWidget: false, rejectedSetters: [], selectionCalls: 0,
    };
    window.__securityInjected = 0;
    window.__copiedSecurityText = [];
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
      writeText: async text => { window.__copiedSecurityText.push(text); },
    } });
    window.__TAURI__ = { core: { invoke: async (name, payload) => {
      const params = payload?.params || {};
      let value;
      switch (name) {
        case "ping": value = { version: "synthetic" }; break;
        case "settings_get": value = { default_profile: identity.profile, default_region: identity.region }; break;
        case "dashboard_get": value = { tiles: [{ id: "aws-cli", widget: "aws-cli", x: 0, y: 0, w: 12, h: 9,
          config: { context: { mode: "pinned", ...identity }, inputs: options.pin
            ? { pinned_cli_commands: [{ ...identity, command }] } : { command } },
        }] }; break;
        case "dashboard_set": value = { ok: true }; break;
        case "settings_set":
          value = fixture.holdSettings ? await new Promise(resolve => { fixture.finishSettings = resolve; }) : { ok: true };
          break;
        case "aws_list_profiles": value = { profiles: [{ name: identity.profile, account_id: identity.account_id,
          region: identity.region, role_name: "SyntheticReadOnly", sso_session: "synthetic-session" }] }; break;
        case "aws_set_account": fixture.selectionCalls++; active = true; value = { ok: true, ...identity }; break;
        case "aws_auth_status": value = { has_context: active, logged_in: active,
          connection_state: active ? "verified" : "disconnected", ...(active ? identity : {}) }; break;
        case "aws_list_pipelines": value = { ok: true, pipelines: [] }; break;
        case "widget_fetch":
          if (fixture.rejectWidget) throw new Error("CB_SYNTHETIC_DIAGNOSTIC_SECRET");
          value = structuredClone(params.widget === "aws-cli" ? fixture.response : { render: "raw_json", data: {} }); break;
        case "audit_tail": value = structuredClone(fixture.audit); break;
        case "policy_get": value = { raw: fixture.policy, valid: true, actions: [], path: "/synthetic/policy.yaml" }; break;
        case "policy_set": value = { raw: params.text, valid: true, actions: [] }; break;
        default: throw new Error("Unexpected synthetic invoke");
      }
      if (fixture.rejectedSetters.includes(name)) {
        value = { ok: false, error_type: "InvalidRequest", error: "CB_SYNTHETIC_DIAGNOSTIC_SECRET" };
      }
      if (["widget_fetch", "aws_list_pipelines", "aws_set_account", "aws_auth_status"].includes(name)) {
        const verified = name !== "aws_auth_status" || active;
        value._request = { id: params.request_id ?? null, audit_id: "audit-synthetic-1",
          outcome: value.ok === false ? "failed" : "succeeded",
          context_id: verified ? "synthetic-context" : null, provider_revision: verified ? "1" : null,
          settings_revision: verified ? "1" : null, profile: verified ? identity.profile : null,
          account_id: verified ? identity.account_id : null, region: verified ? identity.region : null,
          ...value._request };
      }
      return { ...value, _diagnostics: { audit_write_failed: fixture.auditWriteFailed } };
    } } };
  }, { response, options });
  await page.goto("/");
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "online");
  const surface = page.locator('.widget[data-widget="aws-cli"]');
  await expect(surface).toHaveCount(1);
  if (options.pin) await surface.locator(".pipeline-pin-toggle").click();
  else await expect(surface.locator(".aws-cli-rows")).toBeVisible();
  return { surface, remoteRequests, consoleMessages };
}

for (const [name, response] of [
  ["table cells", { render: "table", columns: ["resource"], rows: [{ resource: HOSTILE }] }],
  ["log messages", { render: "log_stream", events: [{ msg: HOSTILE, level: "info" }] }],
  ["execution details", { render: "execution_detail", actions: [{ stage: HOSTILE, action: HOSTILE, summary: HOSTILE, error: HOSTILE }] }],
  ["error chart labels", { render: "errors_chart", hours: 1, rows: [{ stack: HOSTILE, errors: 1 }] }],
  ["raw JSON", { render: "raw_json", data: { resource: HOSTILE } }],
  ["inline errors", { render: "raw_json", data: { error: HOSTILE } }],
  ["permission errors", { render: "permission_denied", action: "logs:StartQuery", reason: HOSTILE }],
]) {
  test(`${name} render hostile resource strings literally`, async ({ page }) => {
    const { surface, remoteRequests } = await boot(page, response);
    const rows = surface.locator(".aws-cli-rows");
    await expect(rows).toContainText(HOSTILE);
    await expect(rows.locator("img, svg, script, [onerror], [onload]")).toHaveCount(0);
    expect(await page.evaluate(() => window.__securityInjected)).toBe(0);
    expect(remoteRequests).toEqual([]);
  });
}

test("only exact supported HTTPS console links get navigation and copy actions", async ({ page }) => {
  const good = [
    "https://console.aws.amazon.com/console/home",
    "https://eu-west-1.console.aws.amazon.com/codesuite/codebuild/projects/synthetic",
    "https://us-east-1.console.aws.amazon.com/cloudwatch/home#logsV2:log-groups",
  ];
  const bad = [
    "javascript:window.__securityInjected=3", "data:text/html,<svg onload=alert(1)>",
    "file:///synthetic/path", "http://console.aws.amazon.com/", "//console.aws.amazon.com/",
    "https://console.aws.amazon.com.evil.invalid/", "https://evil.invalid/?next=console.aws.amazon.com",
    "https://console.aws.amazon.com@evil.invalid/", "https://user@console.aws.amazon.com/",
    "https://console.aws.amazon.com:443/", "https://console.aws.amazon.com:8443/",
    "https://console.aws.amazon.com./", "https://console%2eaws.amazon.com/",
    "https://console.aws.amazon.com\\@evil.invalid/", "https://console.aws.amazon.com/\nsynthetic",
    " https://console.aws.amazon.com/", "https://console.aws.amazon.com/%0a",
    "https://eu-central-1.console.aws.amazon.com/", "https://s3.amazonaws.com/",
  ];
  const actions = [...good, ...bad].map((external_url, i) => ({ action: `action-${i}`, external_url }));
  const { surface, remoteRequests } = await boot(page, { render: "execution_detail", actions });
  const links = surface.locator(".exec-detail a");
  await expect(links).toHaveCount(good.length);
  for (let i = 0; i < good.length; i++) {
    await expect(links.nth(i)).toHaveAttribute("href", good[i]);
    await expect(links.nth(i)).toHaveAttribute("target", "_blank");
    await expect(links.nth(i)).toHaveAttribute("rel", "noopener noreferrer");
  }
  await expect(surface.getByText("External link unavailable: unsupported console URL.", { exact: true })).toHaveCount(bad.length);
  const copy = surface.getByRole("button", { name: "Copy AWS Console link", exact: true });
  await expect(copy).toHaveCount(good.length);
  await copy.first().click();
  await expect.poll(() => page.evaluate(() => window.__copiedSecurityText)).toEqual([good[0]]);
  expect(await page.evaluate(() => window.__securityInjected)).toBe(0);
  expect(remoteRequests).toEqual([]);
});

test("policy highlighting preserves hostile YAML as text", async ({ page }) => {
  const { remoteRequests } = await boot(page);
  await page.locator("#settings-btn").click();
  await page.locator("#policy-editor").fill(`Statement:\n  - Effect: Allow\n    Resource: '${HOSTILE}'\n# ${HOSTILE}`);
  const highlight = page.locator("#policy-highlight");
  await expect(highlight).toContainText(HOSTILE);
  await expect(highlight.locator("img, svg, script, [onerror], [onload]")).toHaveCount(0);
  expect(await page.evaluate(() => window.__securityInjected)).toBe(0);
  expect(remoteRequests).toEqual([]);
});

test("audit failure stays visible and failed reads retain prior entries without raw diagnostics", async ({ page }) => {
  const { consoleMessages } = await boot(page, table, { auditWriteFailed: true });
  await expect(page.locator("#audit-write-warning")).toBeVisible();
  await expect(page.locator("#audit-panel")).not.toHaveClass(/open/);
  await page.evaluate(secret => {
    window.__securityFixture.auditWriteFailed = false; // An older healthy response cannot erase a lost entry.
    window.__securityFixture.audit = { entries: [
      ...["started", "succeeded", "failed", "denied", "cancelled"].map(event => ({
        kind: "request", scope: "application", event, command: "widget_fetch", request_id: "audit-synthetic-1",
      })),
      { kind: "aws", scope: "capability_preflight", service: "logs", operation: "StartQuery", reason: secret },
      { kind: "lifecycle", event: "set_account_failed", raw_config: secret, credentials: secret },
      { kind: "widget", scope: "application", widget: "logs-insights", event: "query_cleanup",
        cleanup_status: "not_confirmed", request_id: "audit-cleanup-fixture-1", failed: 2, message: secret },
    ] };
  }, PRIVATE_MARKER);
  await page.locator("#audit-btn").click();
  const rows = page.locator("#audit-table-wrap");
  await expect(rows.getByText("Succeeded", { exact: true })).toBeVisible();
  await expect(rows.getByText("Cancelled (result discarded)", { exact: true })).toBeVisible();
  await expect(rows).toContainText("Permission check allowed; execution not confirmed");
  const cleanup = rows.locator("tr").filter({ hasText: "Query cleanup: remote stop not confirmed" });
  await expect(cleanup).toContainText("logs-insights");
  await expect(cleanup).toContainText("audit-cleanup-fixture-1");
  await expect(cleanup).toContainText("2 failed");
  await expect(rows).not.toContainText(PRIVATE_MARKER);
  await page.locator("#audit-panel-close").click();
  await page.evaluate(secret => {
    window.__securityFixture.audit = { ok: false, error_type: "AuditReadFailed", error: secret };
  }, PRIVATE_MARKER);
  await page.locator("#audit-btn").click();
  await expect(page.locator("#audit-read-warning")).toBeVisible();
  await expect(rows.getByText("Succeeded", { exact: true })).toBeVisible();
  await expect(rows).not.toContainText("No entries yet.");
  await expect(page.locator("#audit-write-warning")).toBeVisible();
  await expect(page.locator("body")).not.toContainText(PRIVATE_MARKER);
  expect(consoleMessages.join("\n")).not.toContain(PRIVATE_MARKER);
});

test("unexpected bridge errors do not disclose native diagnostic payloads", async ({ page }) => {
  const { surface, consoleMessages } = await boot(page);
  await page.evaluate(() => { window.__securityFixture.rejectWidget = true; });
  await surface.locator(".cli-run-btn").click();
  await expect(surface).toContainText("Desktop request failed. Try again.");
  await expect(page.locator("body")).not.toContainText(PRIVATE_MARKER);
  expect(consoleMessages.join("\n")).not.toContain(PRIVATE_MARKER);
});

test("a failed pinned CLI render cannot receive a success badge", async ({ page }) => {
  const response = { ok: false, render: "raw_json", data: { error: "Synthetic cleanup was not confirmed" },
    _request: { outcome: "failed" } };
  const { surface } = await boot(page, response, { pin: true });
  await expect(surface.locator(".pipeline-pin-status")).toHaveText("Failed");
  await expect(surface.locator(".pipeline-pin-result")).toContainText("Synthetic cleanup was not confirmed");
});

test("failed and partial error charts never claim zero errors", async ({ page }) => {
  const response = { render: "errors_chart", status: "failed", partial: true, rows: [],
    error: "Synthetic query failed", counts: { queried: 2, succeeded: 0, failed: 1, timed_out: 1 } };
  const { surface } = await boot(page, response);
  await expect(surface).toContainText("absence of rows does not establish zero errors");
  await expect(surface).toContainText("0 succeeded, 1 failed, 1 timed out, 2 queried");
  await expect(surface).not.toContainText("No error events");
  await page.evaluate(() => {
    window.__securityFixture.response = { render: "errors_chart", status: "partial", partial: true,
      rows: [{ stack: "synthetic surviving group", errors: 2 }], error: "Some groups failed",
      counts: { queried: 2, succeeded: 1, failed: 1, timed_out: 0 } };
  });
  await surface.locator(".cli-run-btn").click();
  await expect(surface).toContainText("synthetic surviving group");
  await expect(surface).toContainText("1 succeeded, 1 failed");
});

test("validation-rejected settings and policy keep drafts and verified results intact", async ({ page }) => {
  const { surface } = await boot(page);
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("demo-a");
  await expect(page.locator("#policy-path")).toHaveText("/synthetic/policy.yaml");
  const selections = await page.evaluate(() => {
    window.__securityFixture.rejectedSetters = ["settings_set", "policy_set"];
    return window.__securityFixture.selectionCalls;
  });
  await page.locator("#settings-default-profile").fill("draft-profile");
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-status")).toContainText("Changes were rejected");
  await expect(page.locator("#settings-status")).not.toHaveText("Saved.");
  await expect(page.locator("#settings-default-profile")).toHaveValue("draft-profile");
  await expect(page.locator("#settings-save")).toBeEnabled();
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "online");
  await expect(surface.locator(".aws-cli-rows")).toContainText("synthetic resource");
  expect(await page.evaluate(() => window.__securityFixture.selectionCalls)).toBe(selections);
  const draft = "Version: '2012-10-17'\nStatement: []\n# synthetic candidate";
  await page.locator("#policy-editor").fill(draft);
  await page.locator("#policy-save").click();
  await expect(page.locator("#policy-status")).toContainText("Changes were rejected");
  await expect(page.locator("#policy-editor")).toHaveValue(draft);
  await expect(page.locator("#policy-path")).toHaveText("/synthetic/policy.yaml");
  await expect(page.locator("body")).not.toContainText(PRIVATE_MARKER);
});

test("validation-rejected dashboard save is visible and reset keeps the current page", async ({ page }) => {
  const { surface } = await boot(page);
  await page.evaluate(() => {
    window.__securityFixture.rejectedSetters = ["dashboard_set"];
    window.__resetSentinel = "synthetic-page-lifetime";
  });
  await surface.locator(".cli-pin-btn").click();
  await expect(page.locator("#layout-save-warning")).toHaveText("Dashboard changes were not saved. The current layout is still displayed.");
  await page.locator("#reset-layout-btn").click();
  await expect(page.locator("#layout-save-warning")).toHaveText("Layout reset was not saved. The current layout is unchanged.");
  expect(await page.evaluate(() => window.__resetSentinel)).toBe("synthetic-page-lifetime");
  await expect(surface).toHaveCount(1);
  await expect(surface.locator(".pipeline-pin-card")).toHaveCount(1);
  await expect(page.locator("body")).not.toContainText(PRIVATE_MARKER);
});

test("a late accepted settings callback does not replace a newer selection", async ({ page }) => {
  const { surface } = await boot(page);
  await page.locator("#settings-btn").click();
  await expect(page.locator("#settings-default-profile")).toHaveValue("demo-a");
  await page.evaluate(() => { window.__securityFixture.holdSettings = true; });
  await page.locator("#settings-save").click();
  await expect(page.locator("#settings-save")).toBeDisabled();
  const before = await page.evaluate(() => window.__securityFixture.selectionCalls);
  // A distinct connection attempt can reuse the same labels; it must still win.
  await page.evaluate(() => document.querySelector("#account-select").dispatchEvent(new Event("change")));
  await expect.poll(() => page.evaluate(() => window.__securityFixture.selectionCalls)).toBe(before + 1);
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "online");
  await page.evaluate(() => window.__securityFixture.finishSettings({ default_profile: "demo-a", default_region: "eu-west-1" }));
  await expect(page.locator("#settings-save")).toBeEnabled();
  await expect(page.locator("#settings-status")).toHaveText("Saved.");
  expect(await page.evaluate(() => window.__securityFixture.selectionCalls)).toBe(before + 1);
  await expect(page.locator("#auth-status")).toHaveAttribute("data-state", "online");
  await expect(surface.locator(".aws-cli-rows")).toContainText("synthetic resource");
});
