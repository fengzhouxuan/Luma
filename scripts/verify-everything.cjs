// Optional live test. Requires a built Luma and a running Everything client.
// Creates only its own fixtures, prints counts/timing, and removes them in finally.
const assert = require('node:assert/strict');
const cp = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const executable = process.env.LUMA_FILE_SEARCH_EXE;
if (!executable) throw new Error('Set LUMA_FILE_SEARCH_EXE to the built luma.exe');
function query(text, offset = 0, legacy = false) {
  const started = performance.now();
  const result = cp.spawnSync(executable, ['--luma-file-search-worker'], {
    input: JSON.stringify({ query: text, offset, limit: 50, force_legacy: legacy }),
    encoding: 'utf8', windowsHide: true, timeout: 8000, maxBuffer: 16 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  assert.equal(result.status, 0);
  const value = JSON.parse(result.stdout);
  assert.equal(value.state, 'ready', value.message);
  return { ...value, elapsed: Math.round(performance.now() - started) };
}
async function main() {
  const tempRoot = fs.realpathSync.native(os.tmpdir());
  const directory = fs.mkdtempSync(path.join(tempRoot, 'LumaSearch-QA-'));
  try {
    for (let index = 0; index < 52; index++) fs.writeFileSync(path.join(directory, `Luma-QA-${String(index).padStart(2, '0')}.txt`), 'fixture');
    const unicode = path.join(directory, '中文 报价 & 空格.txt');
    fs.writeFileSync(unicode, '中文测试');
    fs.mkdirSync(path.join(directory, '测试 文件夹'));
    const longDirectory = path.join(directory, 'long-'.repeat(35), 'sub-'.repeat(22));
    fs.mkdirSync(longDirectory, { recursive: true });
    fs.writeFileSync(path.join(longDirectory, '长路径.txt'), 'long path');
    const search = `file: "${directory}\\"`;
    let first;
    for (let attempt = 0; attempt < 30; attempt++) {
      first = query(search);
      if (first.total === 54) break;
      await new Promise(resolve => setTimeout(resolve, 300));
    }
    assert.equal(first.total, 54, 'Everything must index all fixtures');
    assert.equal(first.items.length, 50);
    const second = query(search, 50);
    assert.equal(second.items.length, 4);
    const combined = [...first.items, ...second.items];
    assert.equal(new Set(combined.map(item => item.id)).size, 54);
    assert.ok(combined.some(item => item.path === unicode));
    assert.ok(combined.some(item => item.path.length > 260));
    assert.ok(combined.every(item => !item.is_dir));
    const legacy = query(search, 0, true);
    assert.equal(legacy.backend, 'SDK2');
    assert.equal(legacy.total, first.total);
    const folders = query(`folder: "${directory}\\"`);
    assert.ok(folders.items.length >= 3);
    assert.ok(folders.items.every(item => item.is_dir));
    const absent = query('Luma-QA-no-such-file-' + crypto.randomUUID());
    assert.equal(absent.total, 0);
    assert.deepEqual(absent.items, []);
    const probe = query('');
    assert.equal(probe.total, 0);
    assert.deepEqual(probe.items, []);
    console.log(JSON.stringify({ version: first.version, backend: first.backend, files: combined.length, folders: folders.items.length, unicode: true, longPath: true, pagination: true, legacyFallback: true, firstPageMs: first.elapsed, secondPageMs: second.elapsed }));
  } finally {
    const resolved = path.resolve(directory);
    assert.equal(path.dirname(resolved).toLowerCase(), tempRoot.toLowerCase());
    assert.ok(path.basename(resolved).startsWith('LumaSearch-QA-'));
    fs.rmSync(resolved, { recursive: true, force: true });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
