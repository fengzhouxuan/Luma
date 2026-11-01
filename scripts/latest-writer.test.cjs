const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');
const sandbox = { exports: {}, setTimeout, clearTimeout };
vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(__dirname, '../src/latestWriter.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText, sandbox);
const { LatestWriter } = sandbox.exports;

test('coalesces consecutive input changes and flushes the latest value on blur or key release', async () => {
  const saved = [];
  const writer = new LatestWriter(async value => saved.push(value), assert.fail, 10000);
  writer.set(20); writer.set(22); writer.set(24);
  await writer.flush();
  assert.deepEqual(saved, [24]);
});
test('serializes in-flight saves and persists the newest pending value', async () => {
  let release;
  const blocked = new Promise(resolve => { release = resolve; });
  const saved = [];
  const writer = new LatestWriter(async value => { saved.push(value); if (value === 12) await blocked; }, assert.fail, 10000);
  writer.set(12);
  const first = writer.flush();
  writer.set(18); writer.set(24);
  const second = writer.flush();
  assert.deepEqual(saved, [12]);
  release();
  await Promise.all([first, second]);
  assert.deepEqual(saved, [12, 24]);
});
test('reports a failed save and still accepts later keyboard or assistive input', async () => {
  const saved = [], errors = [];
  const writer = new LatestWriter(async value => { if (value === 12) throw Error('disk full'); saved.push(value); }, error => errors.push(error.message), 10000);
  writer.set(12); await writer.flush();
  writer.set(24); await writer.flush();
  assert.deepEqual(errors, ['disk full']);
  assert.deepEqual(saved, [24]);
});
