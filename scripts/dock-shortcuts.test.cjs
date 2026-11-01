const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');
const sandbox = { exports: {} };
vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(__dirname, '../src/dockShortcuts.ts'), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
}).outputText, sandbox);
const { dockShortcutIndex } = sandbox.exports;
const event = overrides => ({ key: '1', code: 'Digit1', altKey: true, ctrlKey: false, shiftKey: false, metaKey: false, repeat: false, isComposing: false, ...overrides });
test('Alt plus digits maps the first nine dock positions including numpad keys', () => {
  for(let digit=1;digit<=9;digit++){
    assert.equal(dockShortcutIndex(event({key:String(digit),code:`Digit${digit}`})),digit-1);
    assert.equal(dockShortcutIndex(event({key:'End',code:`Numpad${digit}`})),digit-1);
  }
  for(const [key,code] of [['0','Digit0'],['a','KeyA'],['ArrowLeft','ArrowLeft']])assert.equal(dockShortcutIndex(event({key,code})),null);
});
test('other modifiers, repeats and IME composition never trigger dock launches', () => {
  for(const overrides of [{altKey:false},{ctrlKey:true},{metaKey:true},{shiftKey:true},{repeat:true},{isComposing:true}])assert.equal(dockShortcutIndex(event(overrides)),null);
});
