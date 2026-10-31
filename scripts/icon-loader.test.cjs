const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');

const sandbox = { exports: {} };
vm.runInNewContext(ts.transpileModule(
  fs.readFileSync(path.join(__dirname, '../src/iconLoader.ts'), 'utf8'),
  { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } },
).outputText, sandbox);
const { IconLoader, iconKey } = sandbox.exports;
const flush = () => new Promise(setImmediate);
const target = (name, shortcut_path = null) => ({ path: `${name}.exe`, shortcut_path });

function harness(concurrency = 2) {
  const requests = [];
  const received = [];
  let now = 0;
  const loader = new IconLoader((app) => new Promise((resolve, reject) => {
    requests.push({ app, resolve, reject });
  }), (key, icon) => received.push({ key, icon }), concurrency, () => now);
  return { loader, requests, received, clock: (value) => { now = value; } };
}

test('limits outstanding work and replaces queued offscreen targets', async () => {
  const { loader, requests } = harness();
  loader.setVisible([target('a'), target('b'), target('offscreen')]);
  await flush();
  assert.deepEqual(requests.map(({ app }) => app.path), ['a.exe', 'b.exe']);
  loader.setVisible([target('c'), target('d')]);
  requests[0].resolve('a-icon');
  await flush();
  assert.deepEqual(requests.map(({ app }) => app.path), ['a.exe', 'b.exe', 'c.exe']);
  requests[1].resolve('b-icon');
  await flush();
  assert.equal(requests[3].app.path, 'd.exe');
  requests[2].resolve('c-icon');
  requests[3].resolve('d-icon');
  await flush();
  loader.setVisible([target('c'), target('d')]);
  await flush();
  assert.equal(requests.length, 4);
});

test('rescan ignores stale results while old requests retain concurrency slots', async () => {
  const { loader, requests, received } = harness(1);
  loader.setVisible([target('same')]);
  await flush();
  loader.reset();
  loader.setVisible([target('same')]);
  await flush();
  assert.equal(requests.length, 1);
  requests[0].resolve('outdated');
  await flush();
  assert.equal(received.length, 0);
  assert.equal(requests.length, 2);
  requests[1].resolve('current');
  await flush();
  assert.equal(received[0].icon, 'current');
});

test('negative results retry after 30 seconds without overwriting good icons', async () => {
  const { loader, requests, received, clock } = harness();
  loader.setVisible([target('retry')]);
  await flush();
  requests[0].resolve(null);
  await flush();
  clock(29_999);
  loader.setVisible([target('retry')]);
  loader.retry();
  await flush();
  assert.equal(requests.length, 1);
  assert.equal(received.length, 0);
  clock(30_000);
  loader.retry();
  await flush();
  assert.equal(requests.length, 2);
  requests[1].resolve('recovered');
  await flush();
  clock(600_000);
  loader.retry();
  await flush();
  assert.equal(requests.length, 2);
  assert.equal(received[0].icon, 'recovered');
});

test('different shortcuts sharing one EXE keep distinct icons', async () => {
  const { loader, requests, received } = harness();
  const first = target('shared', 'profile-a.lnk');
  const second = target('shared', 'profile-b.lnk');
  assert.notEqual(iconKey(first), iconKey(second));
  loader.setVisible([first, second, first]);
  await flush();
  assert.equal(requests.length, 2);
  requests[0].resolve('first');
  requests[1].resolve('second');
  await flush();
  assert.equal(new Set(received.map(({ key }) => key)).size, 2);
});

test('rejections allow the next visible item to load and stop work after reset', async () => {
  const { loader, requests, received } = harness(1);
  loader.setVisible([target('failed'), target('next')]);
  await flush();
  requests[0].reject(new Error('temporary failure'));
  await flush();
  assert.equal(requests[1].app.path, 'next.exe');
  loader.reset();
  requests[1].resolve('stale');
  await flush();
  assert.equal(received.length, 0);
  assert.equal(requests.length, 2);
});
