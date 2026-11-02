const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');
const sandbox = { exports: {} };
vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(__dirname, '../src/fileSearch.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText, sandbox);
const { fileQuery, fileSize, SearchRevision } = sandbox.exports;

test('blank file searches only probe status and type filters cover the whole OR expression', () => {
  for (const filter of ['all', 'files', 'folders']) assert.equal(fileQuery('  ', filter), '');
  assert.equal(fileQuery(' 中文 | report ext:pdf ', 'files'), 'file: <中文 | report ext:pdf>');
  assert.equal(fileQuery('D:\\测试 目录', 'folders'), 'folder: <D:\\测试 目录>');
  assert.equal(fileQuery('ext:pdf | ext:docx', 'all'), 'ext:pdf | ext:docx');
});
test('a slower obsolete response cannot replace a newer query or page', async () => {
  const revisions = new SearchRevision();
  const old = revisions.next();
  const recent = revisions.next();
  let displayed = '';
  await Promise.all([
    new Promise(resolve => setTimeout(() => { if (revisions.isCurrent(old)) displayed = 'old'; resolve(); }, 20)),
    Promise.resolve().then(() => { if (revisions.isCurrent(recent)) displayed = 'recent'; }),
  ]);
  assert.equal(displayed, 'recent');
  revisions.next();
  assert.equal(revisions.isCurrent(recent), false);
});
test('file size distinguishes unknown sizes from real empty files', () => {
  assert.equal(fileSize(null), ''); assert.equal(fileSize(0), '0 B');
  assert.equal(fileSize(1024), '1.0 KB'); assert.equal(fileSize(1024 ** 3), '1.0 GB');
});
