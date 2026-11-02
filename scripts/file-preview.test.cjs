const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const ts=require('typescript');
const sandbox={exports:{}};
vm.runInNewContext(ts.transpileModule(fs.readFileSync(require('node:path').join(__dirname,'../src/filePreview.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText,sandbox);
const {fileShortcut,PreviewQueue,previewError}=sandbox.exports;
const key=(key,extra={})=>({key,altKey:false,metaKey:false,ctrlKey:false,shiftKey:false,repeat:false,isComposing:false,...extra});
test('Space previews only result rows and preserves spaces in search and other controls',()=>{
 assert.equal(fileShortcut(key(' '),'result'),'preview');
 for(const target of ['search','other'])assert.equal(fileShortcut(key(' '),target),null);
 for(const flag of ['isComposing','repeat','ctrlKey','altKey','metaKey','shiftKey'])assert.equal(fileShortcut(key(' ',{[flag]:true}),'result'),null);
 assert.equal(fileShortcut(key('Enter',{shiftKey:true}),'result'),'reveal');
 assert.equal(fileShortcut(key('c',{ctrlKey:true,shiftKey:true}),'search'),'copy');
 assert.equal(fileShortcut(key('ArrowDown',{repeat:true}),'result'),'next');
});
test('rapid selection changes serialize IPC and discard intermediate queued previews',async()=>{
 const sent=[];let release;
 const queue=new PreviewQueue(async id=>{sent.push(id);if(id==='first')await new Promise(resolve=>release=resolve);});
 const first=queue.submit('first');await Promise.resolve();await Promise.resolve();
 const skipped=queue.submit('second');const last=queue.submit('last');release();
 assert.equal(await first,false);assert.equal(await skipped,false);assert.equal(await last,true);
 assert.deepEqual(sent,['first','last']);
});
test('cancelling while QuickLook starts guarantees a final Close and ignores the late result',async()=>{
 const sent=[];let release;
 const queue=new PreviewQueue(async id=>{sent.push(id);if(id)await new Promise(resolve=>release=resolve);});
 const open=queue.submit('file');await Promise.resolve();await Promise.resolve();
 const close=queue.submit(null);release();assert.equal(await open,false);assert.equal(await close,true);
 assert.deepEqual(sent,['file',null]);
});
test('failed IPC does not poison later requests and structured errors expose installation help',async()=>{
 const sent=[];const queue=new PreviewQueue(async id=>{sent.push(id);if(id==='bad')throw Error('offline');});
 await assert.rejects(queue.submit('bad'),/offline/);assert.equal(await queue.submit(null),true);
 assert.deepEqual(sent,['bad',null]);
 assert.equal(previewError({code:'unavailable',message:'install'}).missing,true);
 assert.equal(previewError({code:'error',message:'timeout'}).missing,false);
 assert.equal(previewError('offline').message,'offline');
});
