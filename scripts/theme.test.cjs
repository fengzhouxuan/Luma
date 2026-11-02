const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const ts=require('typescript');
const sandbox={exports:{}};
vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(__dirname,'../src/theme.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText,sandbox);
const {resolveTheme,observeTheme}=sandbox.exports;
const tick=()=>new Promise(resolve=>setImmediate(resolve));
test('legacy preferences stay dark, explicit modes ignore the OS, and system mode follows it',()=>{
 for(const system of [true,false]){
  assert.equal(resolveTheme(undefined,system),'dark');
  assert.equal(resolveTheme('dark',system),'dark');
  assert.equal(resolveTheme('light',system),'light');
 }
 assert.equal(resolveTheme('system',true),'dark');assert.equal(resolveTheme('system',false),'light');
});
test('a late initial theme cannot overwrite a newer system change',async()=>{
 const values=[];let update,finish;let unsubscribed=false;
 const stop=observeTheme(()=>new Promise(resolve=>finish=resolve),async handler=>{update=handler;return()=>unsubscribed=true;},theme=>values.push(theme));
 await tick();update('light');finish('dark');await tick();
 assert.deepEqual(values,['light']);update('dark');assert.deepEqual(values,['light','dark']);
 stop();assert.equal(unsubscribed,true);update('light');assert.deepEqual(values,['light','dark']);
});
test('disposing before subscription completes removes the late listener and ignores its callbacks',async()=>{
 let subscribed,update,removed=false,reads=0;const values=[];
 const stop=observeTheme(async()=>{reads++;return 'dark';},handler=>{update=handler;return new Promise(resolve=>subscribed=resolve);},theme=>values.push(theme));
 stop();subscribed(()=>removed=true);await tick();update('light');
 assert.equal(removed,true);assert.equal(reads,0);assert.deepEqual(values,[]);
});
test('system events received during listener registration take precedence over the initial read',async()=>{
 const values=[];let subscribed;let reads=0;
 const stop=observeTheme(async()=>{reads++;return 'dark';},update=>{update('light');return new Promise(resolve=>subscribed=resolve);},theme=>values.push(theme));
 subscribed(()=>{});await tick();assert.deepEqual(values,['light']);assert.equal(reads,0);stop();
});
test('light body, secondary and muted text retain readable contrast on both glass base colors',()=>{
 const css=fs.readFileSync(path.join(__dirname,'../src/themes.css'),'utf8');
 const luminance=hex=>{const rgb=hex.replace('#','').match(/../g).map(v=>parseInt(v,16)/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4);return .2126*rgb[0]+.7152*rgb[1]+.0722*rgb[2];};
 for(const token of ['--mv-text','--mv-text-secondary','--mv-text-muted']){
  const match=css.match(new RegExp(token+': (#[a-fA-F0-9]{6});'));assert.ok(match,token);
  for(const background of ['#f9faff','#e6f0f7']){const l1=luminance(match[1]),l2=luminance(background);assert.ok((Math.max(l1,l2)+.05)/(Math.min(l1,l2)+.05)>=4.5,token+' on '+background);}
 }
});
