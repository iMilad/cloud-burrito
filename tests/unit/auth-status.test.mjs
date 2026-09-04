import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

// Exercise the production poll handler with deferred bridge replies. The DOM
// fixture exposes only the auth pill and does not load the desktop app or AWS.
const source = readFileSync(new URL('../../frontend/app.js', import.meta.url), 'utf8');
const start = source.indexOf('  async function refreshAuthStatus() {');
const end = source.indexOf('  function startAuthStatusPolling()', start);
assert(start >= 0 && end > start);
const handler = source.slice(start, end);
const openStart = source.indexOf('  function openIdentityPanel() {');
const openEnd = source.indexOf('  function closeIdentityPanel()', openStart);
assert(openStart >= 0 && openEnd > openStart);
const openHandler = source.slice(openStart, openEnd);

function fixture() {
  const label = { textContent: 'auth: verifying new account' };
  const pill = { hidden: false, dataset: { state: 'checking' }, querySelector: () => label };
  const pending = [];
  const rendered = [];
  const openClasses = new Set();
  const panel = { classList: { add: (value) => openClasses.add(value), contains: (value) => openClasses.has(value) }, setAttribute: () => {} };
  const scrim = { classList: { add: () => {} }, hidden: true };
  const context = vm.createContext({
    isTauri: true, currentSelectionId: 1, currentAuthStatusId: 0,
    lastSetAccountResult: null, lastAuthStatus: null,
    topbarState: { profile: 'demo-fixture', accountId: 'acct-current-fixture', region: 'region-fixture' },
    clearInheritedResults: () => rendered.push({ cleared: true }),
    $: (selector) => ({ '#auth-status': pill, '#identity-panel': panel, '#scrim': scrim })[selector] || null,
    formatRemaining: () => '', renderIdentityPanel: (info) => rendered.push(info),
    tauriInvoke: () => new Promise((resolve, reject) => pending.push({ resolve, reject })),
  });
  vm.runInContext(handler + openHandler, context);
  return { context, pill, label, pending, rendered, poll: () => context.refreshAuthStatus() };
}
const verified = (account) => ({ has_context: true, logged_in: true, account_id: account, connection_state: 'verified',
  _request: { id: null, context_id: 'context-fixture', provider_revision: 'provider-fixture', settings_revision: 'settings-fixture',
    profile: 'demo-fixture', account_id: account, region: 'region-fixture' } });

test('opening identity panel cannot revive a cached account after its refresh is discarded', async () => {
  const f = fixture();
  f.context.lastAuthStatus = verified('acct-old-fixture');
  const open = f.context.openIdentityPanel();
  f.context.currentSelectionId++;
  f.pending[0].resolve(verified('acct-old-fixture'));
  await open;
  assert.equal(f.rendered.length, 0);
});

test('old selection success and failure cannot repaint the auth pill', async () => {
  for (const fail of [false, true]) {
    const f = fixture();
    const work = f.poll();
    f.context.currentSelectionId++;
    if (fail) f.pending[0].reject(new Error('fixture old failure'));
    else f.pending[0].resolve(verified('acct-old-fixture'));
    await work;
    assert.equal(f.label.textContent, 'auth: verifying new account');
    assert.equal(f.pill.dataset.state, 'checking');
    assert.equal(f.context.lastAuthStatus, null);
  }
});

test('latest overlapping auth poll wins even if the older reply arrives last', async () => {
  const f = fixture();
  const old = f.poll();
  const fresh = f.poll();
  f.pending[1].resolve(verified('acct-current-fixture'));
  await fresh;
  f.pending[0].resolve(verified('acct-old-fixture'));
  await old;
  assert.equal(f.context.lastAuthStatus.account_id, 'acct-current-fixture');
  assert.match(f.label.textContent, /acct-current-fixture/);
});

test('backend superseded result cannot replace current status', async () => {
  const f = fixture();
  const work = f.poll();
  f.pending[0].resolve({ has_context: false, error_type: 'Superseded' });
  await work;
  assert.equal(f.context.lastAuthStatus, null);
  assert.equal(f.pill.dataset.state, 'checking');
});

test('current expired refresh is shown as a failure, and verifying stays visible', async () => {
  const f = fixture();
  f.context.lastSetAccountResult = { ok: true };
  const work = f.poll();
  f.pending[0].resolve({ has_context: false, logged_in: false, needs_sso_login: true, connection_state: 'failed', error: 'fixture token expired' });
  await work;
  assert.equal(f.pill.dataset.state, 'offline');
  assert.match(f.label.textContent, /expired/);
  assert.equal(f.context.lastSetAccountResult.ok, false);
  assert.deepEqual(f.rendered, [{ cleared: true }]);
  const next = f.poll();
  f.pending[1].resolve({ has_context: false, logged_in: false, connection_state: 'verifying' });
  await next;
  assert.equal(f.pill.dataset.state, 'checking');
  assert.match(f.label.textContent, /verifying/);
});

test('auth status rejects a different verified identity or missing verification metadata', async () => {
  for (const result of [verified('acct-other-fixture'), { ...verified('acct-current-fixture'), _request: null }]) {
    const f = fixture();
    const work = f.poll();
    f.pending[0].resolve(result);
    await work;
    assert.equal(f.context.lastAuthStatus, null);
    assert.equal(f.pill.dataset.state, 'checking');
  }
});
