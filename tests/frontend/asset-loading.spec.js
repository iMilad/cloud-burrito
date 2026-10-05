import { expect, test } from "@playwright/test";

test("parallel pages load all local styles and scripts without dropped connections", async ({ browser, baseURL }) => {
  const contexts = await Promise.all(Array.from({ length: 4 }, () => browser.newContext({ baseURL })));
  const failures = [];
  try {
    const pages = await Promise.all(contexts.map(async context => {
      await context.route("**/*", route => new URL(route.request().url()).origin === new URL(baseURL).origin
        ? route.continue() : route.abort("blockedbyclient"));
      const page = await context.newPage();
      page.on("requestfailed", request => failures.push({
        path: new URL(request.url()).pathname, error: request.failure()?.errorText,
      }));
      return page;
    }));
    await Promise.all(pages.map(page => page.goto("/")));
    expect(failures).toEqual([]);
    for (const page of pages) {
      expect(await page.locator('link[rel="stylesheet"]').evaluateAll(links =>
        links.filter(link => !link.sheet).map(link => new URL(link.href).pathname))).toEqual([]);
      expect(await page.evaluate(() => typeof window.GridStack)).toBe("function");
      expect(await page.evaluate(() => typeof window.CloudBurritoStudio?.init)).toBe("function");
    }
  } finally {
    await Promise.all(contexts.map(context => context.close()));
  }
});
