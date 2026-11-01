const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');
const sandbox = { exports: {} };
vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(__dirname, '../src/dock.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText, sandbox);
const { fixedApps, insertionBefore, adjacentBefore } = sandbox.exports;
const app = id => ({ id, name: id, path: 'C:/same.exe', arguments: id });

test('fixed apps follow saved order, deduplicate IDs and retain distinct launch profiles', () => {
  const catalog = [app('a'), app('b'), app('c')];
  const pins = ['c', 'missing', 'a', 'a', 'b'];
  assert.deepEqual(Array.from(fixedApps(catalog, { pinned_apps: pins }), item => item.id), ['c', 'a', 'b']);
  assert.deepEqual(pins, ['c', 'missing', 'a', 'a', 'b']);
  assert.deepEqual(catalog.map(item => item.id), ['a', 'b', 'c']);
});
test('manual hiding removes an item from the dock while automatic filtering does not override explicit pinning', () => {
  const catalog = [{ ...app('a'), hidden_reason: '附属程序' }, app('b')];
  assert.deepEqual(Array.from(fixedApps(catalog, { pinned_apps: ['a', 'b'], hidden_apps: ['b'] }), item => item.id), ['a']);
  assert.deepEqual(Array.from(fixedApps([], { pinned_apps: ['a'] })), []);
});
test('drop insertion uses item centers, excludes the moving item and supports either end', () => {
  const items = [{ id: 'a', center: 20 }, { id: 'b', center: 60 }, { id: 'c', center: 100 }];
  assert.equal(insertionBefore(items, 0, 'b'), 'a');
  assert.equal(insertionBefore(items, 50, 'b'), 'c');
  assert.equal(insertionBefore(items, 120, 'b'), null);
  assert.equal(insertionBefore([], 50, 'new'), null);
});
test('keyboard moves produce before-ID operations and stop at visible boundaries', () => {
  const items = ['a', 'b', 'c'].map(app);
  assert.equal(adjacentBefore(items, 'b', -1), 'a');
  assert.equal(adjacentBefore(items, 'a', 1), 'c');
  assert.equal(adjacentBefore(items, 'b', 1), null);
  assert.equal(adjacentBefore(items, 'a', -1), undefined);
  assert.equal(adjacentBefore(items, 'c', 1), undefined);
  assert.equal(adjacentBefore(items, 'missing', 1), undefined);
});
