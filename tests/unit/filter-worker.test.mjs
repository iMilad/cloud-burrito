import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../../frontend/app.js', import.meta.url), 'utf8');
const start = source.indexOf('  function createRowMatcher(');
const end = source.indexOf('  function renderTable(', start);
assert(start > 0 && end > start);
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));

function controller() {
  const instances = [];
  class FakeWorker {
    constructor(url) { this.url = String(url); this.messages = []; instances.push(this); }
    postMessage(value) { this.messages.push(value); }
    terminate() { this.terminated = true; }
    reply(data) { this.onmessage({ data }); }
  }
  const context = vm.createContext({ Worker: FakeWorker, URL, document: { baseURI: 'http://127.0.0.1/' }, setTimeout, clearTimeout });
  vm.runInContext(source.slice(start, end), context);
  const owner = { isConnected: true, dataset: {} };
  return { instances, owner, matcher: context.createRowMatcher(owner, ['alpha', 'beta']) };
}

test('worker initialization is reused and current metadata patches keep original row indexes', async () => {
  const f = controller();
  const first = f.matcher.match('alpha');
  await pause(40);
  const worker = f.instances[0];
  assert.equal(worker.url, 'http://127.0.0.1/row-filter-worker.js');
  worker.reply({ type: 'ready' });
  worker.reply({ id: worker.messages.at(-1).id, valid: true, matches: [0] });
  assert.deepEqual((await first).matches, [0]);
  f.matcher.patch([[1, 'updated metadata']]);
  const second = f.matcher.match('updated');
  await pause(40);
  worker.reply({ id: worker.messages.at(-1).id, valid: true, matches: [1] });
  assert.deepEqual((await second).matches, [1]);
  assert.equal(f.instances.length, 1);
  assert.equal(worker.messages.filter(message => message.type === 'init').length, 1);
  assert.equal(worker.messages.filter(message => message.type === 'patch').length, 1);
  f.matcher.dispose();
});

test('a pathological worker is terminated at its deadline without manufacturing empty matches', async () => {
  const f = controller();
  const work = f.matcher.match('(a+)+$');
  await pause(40);
  f.instances[0].reply({ type: 'ready' });
  const result = await work;
  assert.match(result.error, /150 ms/);
  assert.equal(Object.hasOwn(result, 'matches'), false);
  assert.equal(f.instances[0].terminated, true);
  assert.equal(f.owner.dataset.filterPending, 'false');
});

test('superseding and detaching settle old generations and prevent stale worker replies', async () => {
  const f = controller();
  const old = f.matcher.match('alpha');
  await pause(40);
  const firstWorker = f.instances[0];
  firstWorker.reply({ type: 'ready' });
  const next = f.matcher.match('beta');
  assert.equal(await old, null);
  assert.equal(firstWorker.terminated, true);
  firstWorker.reply({ id: 1, valid: true, matches: [0] });
  await pause(40);
  f.owner.isConnected = false;
  f.owner._disposeRender();
  assert.equal(await next, null);
  assert.equal(f.instances[1].terminated, true);
  assert.equal(f.owner.dataset.filterPending, 'false');
});

test('the actual worker preserves regex semantics, literal fallback, and row patch identity', () => {
  const outputs = [];
  const worker = vm.createContext({ self: { postMessage: result => outputs.push(result) } });
  vm.runInContext(readFileSync(new URL('../../frontend/row-filter-worker.js', import.meta.url), 'utf8'), worker);
  const send = data => worker.self.onmessage({ data });
  send({ type: 'init', values: ['prefix-prod-suffix', 'literal[fixture', 'OTHER'] });
  send({ type: 'match', id: 1, query: '.*prod.*' });
  assert.equal(outputs.at(-1).valid, true);
  assert.deepEqual(Array.from(outputs.at(-1).matches), [0]);
  send({ type: 'match', id: 2, query: '[' });
  assert.equal(outputs.at(-1).valid, false);
  assert.deepEqual(Array.from(outputs.at(-1).matches), [1]);
  send({ type: 'patch', entries: [[2, 'new-prod-value']] });
  send({ type: 'match', id: 3, query: 'PROD' });
  assert.deepEqual(Array.from(outputs.at(-1).matches), [0, 2]);
});
