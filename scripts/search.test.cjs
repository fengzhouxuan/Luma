const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');
const sandbox = { exports: {}, require };
vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(__dirname, '../src/search.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText, sandbox);
const { rankApps, matchScore } = sandbox.exports;
const app = (id, name) => ({ id, name, path: `C:/${id}.exe` });

test('exact and prefix results outrank pinned substring matches', () => {
  const items = [app('contains', 'My Code Tools'), app('prefix', 'Code Editor'), app('exact', 'Code')];
  const ranked = rankApps(items, 'code', { pinned_apps: ['contains'] });
  assert.deepEqual(Array.from(ranked, ({ id }) => id), ['exact', 'prefix', 'contains']);
  assert.equal(items[0].id, 'contains');
});
test('supports Chinese, full pinyin, initials and separated words', () => {
  assert.ok(Number.isFinite(matchScore('微信', 'weixin')));
  assert.ok(Number.isFinite(matchScore('微信', 'wx')));
  assert.ok(matchScore('微信', '微信') < matchScore('企业微信', '微信'));
  assert.ok(Number.isFinite(matchScore('Visual Studio Code', 'visual code')));
  assert.ok(Number.isFinite(matchScore('Visual Studio Code', 'vsc')));
  assert.equal(matchScore('微信', 'doesnotexist'), Infinity);
});
test('pins, frequency and recent usage order empty searches', () => {
  const items = [app('a', 'A'), app('b', 'B'), app('c', 'C'), app('d', 'D')];
  const ranked = rankApps(items, '', { pinned_apps: ['d'], usage: { a: { count: 2, last_launch: 30 }, b: { count: 10, last_launch: 20 }, c: { count: 2, last_launch: 40 } } });
  assert.deepEqual(Array.from(ranked, ({ id }) => id), ['d', 'b', 'c', 'a']);
});
test('same-name apps retain distinct identities and stable ordering', () => {
  const ranked = rankApps([app('work', 'Editor'), app('home', 'Editor')], 'editor');
  assert.deepEqual(Array.from(ranked, ({ id }) => id), ['home', 'work']);
});
