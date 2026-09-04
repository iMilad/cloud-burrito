import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
const source = readFileSync(new URL('../../frontend/app.js', import.meta.url), 'utf8');
const start = source.indexOf('  let auditTimer = null;');
const end = source.indexOf('  // ===== Searchable top bar pickers', start);
assert(start > 0 && end > start);
const rows = (start, count) => Array.from({ length: count }, (_, i) => ({ kind: 'request', command: `synthetic-${start + i}` }));
function fixture() {
  const calls = []; const timers = new Map(); let timerId = 0;
  const panel = { visible: true, getAttribute: () => panel.visible ? 'false' : 'true' };
  const warning = { textContent: '', hidden: true };
  const nodes = { '#audit-panel': panel, '#audit-read-warning': warning, '#audit-table-wrap': {}, '#audit-page-status': {} };
  const document = { hidden: false };
  const context = vm.createContext({ isTauri: true, document, $: id => nodes[id],
    setTimeout: callback => { timers.set(++timerId, callback); return timerId; }, clearTimeout: id => timers.delete(id),
    hidePanel: () => { panel.visible = false; }, showPanel: () => { panel.visible = true; },
    tauriInvoke: (command, args) => new Promise((resolve, reject) => calls.push({ command, args, resolve, reject })),
  });
  vm.runInContext(source.slice(start, end) + '\nrenderAuditEntries = () => {};', context);
  return { calls, timers, context, nodes, panel, document, warning,
    state: () => JSON.parse(vm.runInContext('JSON.stringify({entries:auditEntries,cursor:auditCursor,pending:auditReadPending})', context)),
    refresh: () => context.refreshAudit() };
}
test('audit reads never overlap; cursor pages append with a hard300-entry retained bound', async () => {
  const f = fixture(); const first = f.refresh(); await f.refresh();
  assert.equal(f.calls.length, 1);
  assert.deepEqual(JSON.parse(JSON.stringify(f.calls[0].args.params)), { limit: 300 });
  f.calls[0].resolve({ entries: rows(0, 300), cursor: 'synthetic-cursor-1', limited: true }); await first;
  const second = f.refresh(); assert.equal(f.calls[1].args.params.cursor, 'synthetic-cursor-1');
  f.calls[1].resolve({ entries: rows(300, 20), cursor: 'synthetic-cursor-2', has_more: true }); await second;
  assert.equal(f.state().entries.length, 300); assert.equal(f.state().entries[0].command, 'synthetic-20');
  assert.equal(f.state().entries.at(-1).command, 'synthetic-319');
});
test('replacement resets rows and failed reads keep the last confirmed rows and cursor', async () => {
  const f = fixture(); let next = f.refresh(); f.calls[0].resolve({ entries: rows(0, 2), cursor: 'old' }); await next;
  next = f.refresh(); f.calls[1].reject(Error('private synthetic failure')); await next;
  assert.equal(f.state().cursor, 'old'); assert.equal(f.state().entries.length, 2);
  assert.match(f.warning.textContent, /Previously displayed entries are unchanged/);
  next = f.refresh(); f.calls[2].resolve({ entries: rows(9, 1), cursor: 'new', reset: true }); await next;
  assert.deepEqual(f.state().entries, rows(9, 1)); assert.equal(f.warning.hidden, true);
});
test('hidden panels do not poll and a completion after closing cannot replace the displayed page', async () => {
  const f = fixture(); f.document.hidden = true; await f.refresh(); assert.equal(f.calls.length, 0);
  f.document.hidden = false; const pending = f.refresh(); f.context.closeAuditPanel();
  f.calls[0].resolve({ entries: rows(0, 1), cursor: 'stale' }); await pending;
  assert.equal(f.state().entries.length, 0); assert.equal(f.state().cursor, null); assert.equal(f.timers.size, 0);
  await f.refresh(); assert.equal(f.calls.length, 1);
});
test('cursorless legacy snapshots replace rather than duplicate and oversized pages fail closed', async () => {
  const f = fixture(); let next = f.refresh(); f.calls[0].resolve({ entries: rows(0, 2) }); await next;
  next = f.refresh(); f.calls[1].resolve({ entries: rows(0, 2) }); await next;
  assert.equal(f.state().entries.length, 2);
  next = f.refresh(); f.calls[2].resolve({ entries: rows(0, 1001), cursor: 'oversized' }); await next;
  assert.equal(f.state().entries.length, 2); assert.equal(f.state().cursor, null); assert.equal(f.warning.hidden, false);
});
