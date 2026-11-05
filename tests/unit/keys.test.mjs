import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../../frontend/keys.js', import.meta.url), 'utf8');
function keymap(times) {
  const context = vm.createContext({});
  vm.runInContext(source, context);
  let index = 0;
  return context.CloudBurritoKeys.createKeyMap({ now: () => times[Math.min(index++, times.length - 1)] });
}
const key = (k, extra = {}) => ({ key: k, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, typing: false, ...extra });

test('vim keys map to named actions', () => {
  const map = keymap([0]);
  const pairs = [['j', 'down'], ['k', 'up'], ['ArrowDown', 'down'], ['g', 'top'], ['G', 'bottom'], ['Enter', 'open'],
    ['l', 'open'], ['Escape', 'back'], ['h', 'back'], ['/', 'filter'], ['r', 'refresh'], ['R', 'refresh-all'],
    ['y', 'copy'], ['o', 'console'], ['n', 'next-error'], ['N', 'previous-error'], ['a', 'add'], ['e', 'edit'],
    ['J', 'move-down'], ['K', 'move-up'], ['?', 'help'], [':', 'command'], ['s', 'save'], [',', 'settings'], ['Tab', 'focus']];
  for (const [k, action] of pairs) assert.equal(map.action(key(k)), action, k);
  assert.equal(map.action(key('x')), null);
});

test('dd removes only when both presses are close together', () => {
  assert.equal(keymap([0, 100]).action(key('d')), null);
  const quick = keymap([0, 100]);
  quick.action(key('d'));
  assert.equal(quick.action(key('d')), 'remove');
  const slow = keymap([0, 2000]);
  slow.action(key('d'));
  assert.equal(slow.action(key('d')), null);
  const broken = keymap([0, 50, 100]);
  broken.action(key('d')); broken.action(key('j'));
  assert.equal(broken.action(key('d')), null);
});

test('text fields keep their keys except Escape, and command shortcut needs a modifier', () => {
  const map = keymap([0]);
  assert.equal(map.action(key('j', { typing: true })), null);
  assert.equal(map.action(key('Escape', { typing: true })), 'back');
  assert.equal(map.action(key('k', { metaKey: true })), 'command');
  assert.equal(map.action(key('K', { ctrlKey: true })), 'command');
  assert.equal(map.action(key('r', { metaKey: true })), null);
  assert.equal(map.action(key('j', { altKey: true })), null);
});
