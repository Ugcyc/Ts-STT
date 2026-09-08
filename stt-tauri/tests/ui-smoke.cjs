const {chromium}=require('playwright');
const {spawn}=require('node:child_process');
const path=require('node:path');
let server; let browser;
(async()=>{
server=spawn(process.execPath,[path.join(__dirname,'../node_modules/vite/bin/vite.js'),'--host','127.0.0.1','--port','1421','--strictPort'],{cwd:path.join(__dirname,'..'),stdio:'ignore'});
for(let i=0;i<100;i++){try{if((await fetch('http://127.0.0.1:1421')).ok)break;}catch{} if(i===99)throw Error('Test server did not start'); await new Promise(r=>setTimeout(r,100));}
browser=await chromium.launch({headless:true,channel:process.env.TS_STT_BROWSER_CHANNEL || undefined});const page=await browser.newPage({viewport:{width:820,height:700}});const errors=[];page.on('pageerror',e=>errors.push(String(e)));
await page.addInitScript(()=>{
const config={hotkey:'CommandOrControl+Shift+Space',mode:'toggle',theme:'dark',output_mode:'paste',mic_device:null,language:'en-US',streaming:true,live_typing:false,live_update_ms:560,key_delay_ms:0,model_keep:'always',spoken_punctuation:false,silence_timeout_secs:60,log_transcripts:false,model_path:null,protect_destination:false,stable_live_text:false,live_preview:false,vad_enabled:false,auto_stop_enabled:false,replacements_enabled:false,replacements:[],history_enabled:false,history_limit:20};
const callbacks=new Map(),listeners=new Map();let next=1;
window.__emit=(event,payload)=>{for(const [id,l] of listeners)if(l.event===event)callbacks.get(l.handler)?.({event,id,payload});};
window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback(fn){const id=next++;callbacks.set(id,fn);return id;},unregisterCallback(id){callbacks.delete(id);},async invoke(cmd,args){
 if(cmd==='get_initial_data')return{config,input_devices:['0: Microphone'],model_path:'models',model_exists:true,status:'Idle',history:[],last_transcript:null,protected_insertion_supported:true};
 if(cmd==='plugin:event|listen'){const id=next++;listeners.set(id,args);return id;}
 if(cmd==='plugin:event|unlisten'){listeners.delete(args.eventId);return;}
 if(cmd==='plugin:global-shortcut|is_registered')return true;
 if(cmd==='save_config'){window.__saved=structuredClone(args.config);return args.config;}
 if(cmd==='get_transcript_history')return[];
 if(cmd==='get_status')return 'Idle';
 if(cmd==='copy_transcript'){window.__copied=true;return;}
 return null;
}};
});
await page.goto('http://127.0.0.1:1421');await page.getByRole('button',{name:'Open settings'}).click();
await page.getByText('No replacements added.',{exact:true}).waitFor();
if(await page.getByRole('switch',{name:'Stop after silence'}).getAttribute('aria-checked')!=='false')throw Error('Auto stop default not off');
if(!(await page.getByRole('switch',{name:'Stop after silence'}).isDisabled()))throw Error('Auto stop must require VAD');
await page.getByRole('switch',{name:'Filter long silences'}).click();await page.getByRole('switch',{name:'Stop after silence'}).click();await page.getByLabel('Silence timeout (seconds)').fill('120');
await page.getByRole('button',{name:'Add replacement',exact:true}).click();await page.getByLabel('Heard phrase 1',{exact:true}).fill('test phrase');await page.getByLabel('Replacement 1',{exact:true}).fill('Correct phrase');await page.getByRole('switch',{name:'Apply replacements'}).click();
await page.getByRole('button',{name:'Save changes',exact:true}).click();const saved=await page.evaluate(()=>window.__saved);if(saved.silence_timeout_secs!==120||saved.replacements.length!==1||!saved.replacements_enabled)throw Error('Settings not saved');
await page.evaluate(()=>window.__emit('dictation-completed',{raw_text:'Recovered words',final_text:'Recovered words',seconds:2,warning:'Paste failed; transcript retained.'}));
await page.getByRole('button',{name:'Close',exact:true}).click();await page.getByRole('alert').waitFor();
if(process.env.TS_STT_SCREENSHOT_DIR)await page.screenshot({path:path.join(process.env.TS_STT_SCREENSHOT_DIR,'ts-stt-compact.png')});await page.getByRole('button',{name:'Open settings'}).click();await page.getByRole('button',{name:'Copy last',exact:true}).click();if(!await page.evaluate(()=>window.__copied))throw Error('Recovery copy failed');
await page.getByText('Your phrase replacements',{exact:true}).scrollIntoViewIfNeeded();if(process.env.TS_STT_SCREENSHOT_DIR)await page.screenshot({path:path.join(process.env.TS_STT_SCREENSHOT_DIR,'ts-stt-settings.png')});
if(errors.length)throw Error(errors.join('\n'));console.log('PASS: optional defaults, VAD dependency, configurable timeout, empty replacement list/editing/save, failed-paste recovery, visible compact errors');
})().catch(e=>{console.error(e);process.exitCode=1}).finally(async()=>{if(browser)await browser.close();if(server)server.kill();});
