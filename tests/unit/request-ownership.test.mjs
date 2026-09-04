import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

// Exercise the production ownership boundary; browser specs exercise the real
// renderers and controls. No native adapter, personal configuration or AWS.
const source = readFileSync(new URL('../../frontend/app.js', import.meta.url), 'utf8');
const start = source.indexOf('  let nextOwnedRequestId = 0;');
const end = source.indexOf('  function clearWidgetResults(', start);
const resultHelpersStart = source.indexOf('  function resultHostForRequest(', start);
const fetchIntoStart = source.indexOf('  async function fetchWidgetInto(', resultHelpersStart);
assert(start >= 0 && resultHelpersStart > start && fetchIntoStart > resultHelpersStart && end > fetchIntoStart);
// Keep these cases focused on the real request-ownership code. Result DOM and
// freshness decorators are exercised through the production browser bridge.
const handlers = source.slice(start, resultHelpersStart) + source.slice(fetchIntoStart, end);

function fixture() {
  const pending = [];
  const cancellations = [];
  const rendered = [];
  const errors = [];
  const context = vm.createContext({
    currentSelectionId: 1, isTauri: true, settingsStorageReady: true, discoveryReady: true, lastSetAccountResult: { ok: true },
    topbarState: { profile: 'demo-fixture', accountId: 'acct-a-fixture', region: 'region-fixture' },
    contextForTile: node => node.pin ? { mode: 'pinned', ...node.pin } : { mode: 'inherit' },
    contextPayloadForTile: node => node.pin ? { mode: 'pinned', ...node.pin } : { mode: 'inherit' },
    contextOverrideFromElement: () => null,
    beginResultRequest: () => {},
    disposeRenderTree: () => {},
    tauriInvoke: (command, args) => {
      if (command === 'request_cancel') {
        cancellations.push(args.params.request_id);
        return Promise.resolve({ ok: true, cancelled_locally: true, cleanup_confirmed: false });
      }
      return new Promise((resolve, reject) => pending.push({ command, args, resolve, reject }));
    },
    dispatchRender: (node, result) => rendered.push({ node, result }),
    renderError: (node, error) => errors.push({ node, error }),
  });
  vm.runInContext(handlers, context);
  const node = (parentNode = null, pin = null) => ({ isConnected: true, parentNode, pin,
    closest: () => pin ? {} : null });
  const reply = (entry, label = 'fixture', fields = {}) => ({ render: 'table', columns: ['label'], rows: [{ label }],
    _request: { id: entry.args.params.request_id, context_id: 'context-fixture', provider_revision: 'provider-fixture',
      settings_revision: 'settings-fixture', profile: 'demo-fixture', account_id: 'acct-a-fixture', region: 'region-fixture', ...fields } });
  return { context, pending, cancellations, rendered, errors, node, reply,
    run: owner => context.fetchWidgetInto(owner, 'cfn-stacks', {}) };
}

test('a cached verified selection cannot start inherited work before fresh discovery', async () => {
  const f = fixture();
  f.context.discoveryReady = false;
  await f.run(f.node());
  assert.equal(f.pending.length, 0);
  assert.equal(f.rendered.length, 0);
  assert.equal(f.errors.length, 1);
});

test('A to B to A discards an old result even when labels match again', async () => {
  const f = fixture();
  const work = f.run(f.node());
  f.context.currentSelectionId += 2;
  f.pending[0].resolve(f.reply(f.pending[0], 'old-a'));
  await work;
  assert.equal(f.rendered.length, 0);
  assert.equal(f.errors.length, 0);
});

test('latest owner request wins, including stale failures', async () => {
  for (const fail of [false, true]) {
    const f = fixture(); const owner = f.node();
    const old = f.run(owner); const fresh = f.run(owner);
    f.pending[1].resolve(f.reply(f.pending[1], 'fresh'));
    await fresh;
    if (fail) f.pending[0].reject(new Error('synthetic stale failure'));
    else f.pending[0].resolve(f.reply(f.pending[0], 'old'));
    await old;
    assert.deepEqual(f.rendered.map(item => item.result.rows[0].label), ['fresh']);
    assert.equal(f.errors.length, 0);
    assert.equal(owner._resultContext.id, f.pending[1].args.params.request_id);
  }
});

test('detached owner and reconfigured ancestor cannot receive late completion', async () => {
  for (const detach of [false, true]) {
    const f = fixture(); const parent = f.node(); const owner = f.node(parent);
    const work = f.run(owner);
    if (detach) owner.isConnected = false;
    else f.context.invalidateRequests(parent);
    f.pending[0].resolve(f.reply(f.pending[0]));
    await work;
    assert.equal(f.rendered.length, 0);
  }
});

test('new parent request invalidates pending detail while independent siblings remain current', async () => {
  const f = fixture(); const parent = f.node(); const detail = f.node(parent); const sibling = f.node();
  const oldDetail = f.run(detail); const independent = f.run(sibling);
  f.context.beginOwnedRequest(parent);
  f.pending.forEach(entry => entry.resolve(f.reply(entry)));
  await Promise.all([oldDetail, independent]);
  assert.equal(f.rendered.length, 1);
  assert.equal(f.rendered[0].node, sibling);
});

test('explicit pin survives topbar selection but not configuration invalidation', async () => {
  const pin = { profile: 'demo-fixture', account_id: 'acct-a-fixture', region: 'region-fixture' };
  const f = fixture(); const owner = f.node(null, pin);
  const work = f.run(owner); f.context.currentSelectionId++;
  f.pending[0].resolve(f.reply(f.pending[0])); await work;
  assert.equal(f.rendered.length, 1);
  const next = f.run(owner);
  vm.runInContext('configurationGeneration++', f.context);
  f.pending[1].resolve(f.reply(f.pending[1])); await next;
  assert.equal(f.rendered.length, 1);
});

test('missing or mismatched response metadata never becomes rendered data', async () => {
  for (const fields of [{ id: 'wrong-fixture' }, { account_id: 'acct-b-fixture' }, { context_id: null }, { provider_revision: null }, { settings_revision: '' }]) {
    const f = fixture(); const work = f.run(f.node());
    f.pending[0].resolve(f.reply(f.pending[0], 'forbidden-fixture', fields));
    await work;
    assert.equal(f.rendered.length, 0);
    assert.equal(f.errors.length, 1);
  }
});

test('preverification errors retain the owned request id without claiming a verified context', async () => {
  const f = fixture(); const work = f.run(f.node()); const entry = f.pending[0];
  entry.resolve({ ok: false, error: 'synthetic verification failure', _request: {
    id: entry.args.params.request_id, context_id: null, provider_revision: null, settings_revision: null,
    profile: null, account_id: null, region: null } });
  await work;
  assert.equal(f.rendered.length, 1);
  assert.equal(f.rendered[0].result._request.context_id, null);
});

test('partial table errors still require a verified data context', async () => {
  const f = fixture(); const work = f.run(f.node()); const entry = f.pending[0];
  entry.resolve({ ...f.reply(entry, 'partial-fixture', { context_id: null }), error: 'synthetic partial failure' });
  await work;
  assert.equal(f.rendered.length, 0);
  assert.equal(f.errors.length, 1);
});

test('unverified inherited requests settle without crossing the bridge', async () => {
  const f = fixture(); f.context.lastSetAccountResult = { ok: false };
  await f.run(f.node());
  assert.equal(f.pending.length, 0);
  assert.equal(f.rendered.length, 0);
  assert.equal(f.errors.length, 1);
});

test('superseding an owner cancels only its outstanding subscriber before dispatching the next one', async () => {
  const f = fixture(); const owner = f.node(); const sibling = f.node();
  const old = f.run(owner); const independent = f.run(sibling); const fresh = f.run(owner);
  assert.deepEqual(f.cancellations, [f.pending[0].args.params.request_id]);
  assert.equal(f.pending.length, 3);
  assert.equal(vm.runInContext('pendingOwnedRequests.size', f.context), 2);
  f.pending.forEach(entry => entry.resolve(f.reply(entry)));
  await Promise.all([old, independent, fresh]);
  assert.equal(f.rendered.length, 2);
  assert.equal(vm.runInContext('pendingOwnedRequests.size', f.context), 0);
});

test('explicit cancel keeps the mounted owner current so its cleanup outcome can arrive', async () => {
  const f = fixture(); const owner = f.node(); const work = f.run(owner);
  const request = owner._ownedRequest;
  await f.context.cancelOwnedRequest(request);
  await f.context.cancelOwnedRequest(request);
  assert.deepEqual(f.cancellations, [request.id]);
  assert.equal(request.current(), true);
  assert.equal(vm.runInContext('pendingOwnedRequests.size', f.context), 0);
  f.pending[0].resolve({ ...f.reply(f.pending[0]), ok: false, error_type: 'QueryCancelled',
    cleanup: { status: 'not_confirmed', remote_queries_may_still_run: true } });
  await work;
  assert.equal(f.rendered.length, 1);
  assert.equal(f.rendered[0].result.cleanup.status, 'not_confirmed');
});

test('cancellation before bridge registration prevents dispatch and returns an owned cancelled result', async () => {
  const f = fixture(); const owner = f.node();
  const request = f.context.beginOwnedRequest(owner);
  await f.context.cancelOwnedRequest(request);
  const result = await f.context.fetchWidgetData('cfn-stacks', {}, owner, null, request);
  assert.equal(f.pending.length, 0);
  assert.equal(f.cancellations.length, 0);
  assert.equal(result._request.id, request.id);
  assert.equal(result._request.outcome, 'cancelled');
  assert.equal(request.accept(result), true);
});

test('the pending registry is bounded and removing a common ancestor recovers every slot', async () => {
  const f = fixture(); const parent = f.node();
  const work = Array.from({ length: 257 }, () => f.run(f.node(parent)));
  assert.equal(f.pending.length, 256);
  assert.equal(vm.runInContext('pendingOwnedRequests.size', f.context), 256);
  f.context.invalidateRequests(parent);
  assert.equal(f.cancellations.length, 256);
  assert.equal(new Set(f.cancellations).size, 256);
  assert.equal(vm.runInContext('pendingOwnedRequests.size', f.context), 0);
  f.pending.forEach(entry => entry.resolve(f.reply(entry)));
  await Promise.all(work);
  assert.equal(f.rendered.length, 0);
});

test('request IDs include a page nonce and remain bounded ASCII across independent pages', () => {
  const a = fixture(), b = fixture();
  const first = a.context.beginOwnedRequest(a.node()).id;
  const second = b.context.beginOwnedRequest(b.node()).id;
  assert.notEqual(first, second);
  assert.match(first, /^ui-[a-z0-9]+-[a-z0-9]+-1$/);
  assert.ok(first.length <= 80);
});
