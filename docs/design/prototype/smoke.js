const fs=require('fs'),vm=require('vm'),assert=require('assert');
const code=['data.js','logic.js','views_a.js','views_b.js','views_c.js'].map(f=>fs.readFileSync(__dirname+'/'+f,'utf8')).join('\n');
const ctx=vm.createContext({console,Date,Math,JSON,Object,Array,Set,Map,String,Number,Promise,setTimeout:()=>0});
vm.runInContext(code,ctx); const D=ctx.__DE; const S=()=>D.S_get();
let n=0; const T=(name,fn)=>{ D.reset(); try{fn(); n++; console.log('ok  ',name);}catch(e){console.log('FAIL',name,'\n    ',(e.stack||e.message).split('\n').slice(0,3).join('\n     ')); process.exitCode=1;} };
const html=()=>D.render();
const clean=(h,ctxName)=>{ for(const bad of ['undefined','NaN','[object','null<','>null']) assert(!h.includes(bad), ctxName+' contains '+bad+': '+h.slice(Math.max(0,h.indexOf(bad)-80),h.indexOf(bad)+40).replace(/\n/g,' ')); };
const go=(o)=>D.ACT.go(o);

T('every screen renders cleanly',()=>{
  ['next','env','workspace','audit','settings','ideas'].forEach(name=>{ go({name}); clean(html(),name); });
  ['pool','mine','active','parked','awaiting','returned','done','all'].forEach(g=>{ go({name:'tickets',group:g}); clean(html(),'tickets/'+g); });
});
T('every tab of every ticket renders cleanly',()=>{
  S().tickets.forEach(t=>['overview','review','test','ship','timeline'].forEach(tab=>{ go({name:'ticket',key:t.key,tab}); const h=html(); clean(h,t.key+'/'+tab); assert(h.includes(t.key)); }));
});
T('ticket overview shows description, acceptance, comments, attachments, links and marks unseen ones',()=>{
  go({name:'ticket',key:'PROJ-142',tab:'overview'}); const h=html();
  assert(h.includes('Steps to reproduce')&&h.includes('Acceptance criteria')&&h.includes('Three customers hit this')&&h.includes('network-trace.har')&&h.includes('Session cookie SameSite'));
  assert(h.includes('mention me'),'mention of you is highlighted'); assert(h.includes('>new<'),'unseen comments flagged on first open');
  assert.strictEqual(D.tk('PROJ-142').seenN,3,'opening the overview marks comments seen');
  go({name:'next'}); go({name:'ticket',key:'PROJ-142',tab:'overview'}); assert(!html().includes('>new<'),'no new flags on second open');
});
T('left and right sidebars are present on every route; right shows ticket facts',()=>{
  go({name:'ticket',key:'PROJ-139',tab:'overview'}); const h=html();
  assert(h.includes('class="left"')&&h.includes('class="right"')&&h.includes('Fix version')&&h.includes('2.13.1')&&h.includes('Pull requests')&&h.includes('HOTFIX'));
  go({name:'next'}); assert(html().includes('Claim queue')&&html().includes('Sync'));
});
T('clicking through the whole flow with the real handlers (UI level)',()=>{
  const sug=(id)=>D.ACT.sug({id}); const ids=()=>D.visibleSuggestions().map(s=>s.id);
  assert(ids().includes('PROJ-142:start_review')); sug('PROJ-142:start_review'); assert.strictEqual(D.tk('PROJ-142').local.status,'reviewing');
  assert.strictEqual(S().ui.route.tab,'review'); clean(html(),'review tab');
  D.ACT.markReviewed({key:'PROJ-142'}); assert(D.tk('PROJ-142').reviewed);
  assert(ids().includes('PROJ-142:activate')); sug('PROJ-142:activate'); assert.strictEqual(D.tk('PROJ-142').local.status,'active');
  assert(html().includes('ACTIVE')&&html().includes('overlay on web'),'now bar shows active ticket and overlay');
  go({name:'ticket',key:'PROJ-142',tab:'test'}); assert(html().includes('Test overlay applied in web'));
  D.tk('PROJ-142').checklist.forEach((_,i)=>D.ACT.toggleChk({key:'PROJ-142',i:String(i)}));
  assert(ids().includes('PROJ-142:integrate_ready')); sug('PROJ-142:integrate_ready'); assert(D.tk('PROJ-142').prep);
  assert(html().includes('Overlaps with')&&html().includes('PROJ-127'),'overlap warning shown');
  assert(ids().includes('PROJ-142:push_ready')); sug('PROJ-142:push_ready');
  let sh=S().ui.sheet; assert(sh&&sh.kind==='confirm'&&sh.risk==='high'&&sh.need==='PROJ-142'); let h=html(); assert(h.includes('HIGH RISK')&&h.includes('git push origin')&&h.includes('refs/heads/uat')&&h.includes('disabled'));
  D.runConfirm(); assert.strictEqual(D.tk('PROJ-142').local.status,'active','no push without the typed key');
  D.TEXTINPUT('typed:0','PROJ-14'); D.runConfirm(); assert.strictEqual(D.tk('PROJ-142').merges.length,0,'partial key does not confirm');
  D.TEXTINPUT('typed:0','proj-142'); D.runConfirm();
  assert.strictEqual(D.tk('PROJ-142').local.status,'integrated'); assert.strictEqual(S().ui.sheet.kind,'result'); assert(html().includes('Pushed')&&html().includes('Integrated'));
  assert(S().audit.some(a=>a.action==='git.push_uat.attempted')&&S().audit.some(a=>a.action==='git.push_uat'&&a.outcome==='success'),'audited before and after');
  D.ACT.closeSheet(); go({name:'ticket',key:'PROJ-142',tab:'ship'}); D.advance(9000); clean(html(),'ship after deploy');
  sug('PROJ-142:compose'); const d=D.draftOf(D.tk('PROJ-142'),'comment'); assert(d); assert(html().includes('pipeline #'));
  D.TEXTINPUT('draft:PROJ-142:'+d.id,'Deployed to alpha (edited).'); assert(D.draftOf(D.tk('PROJ-142'),'comment').body.includes('edited'));
  sug('PROJ-142:post'); sh=S().ui.sheet; assert(sh.kind==='confirm'&&sh.risk==='medium'&&!sh.need&&sh.payload.join('').includes('edited')); D.runConfirm();
  assert(D.tk('PROJ-142').comments.some(c=>c.who==='You'),'posted comment appears in the ticket comments');
  sug('PROJ-142:transition'); D.runConfirm(); assert.strictEqual(D.tk('PROJ-142').jira,'Alpha Testing');
  D.SIM.signOff('PROJ-142'); sug('PROJ-142:approve:api-client#212'); D.runConfirm(); sug('PROJ-142:approve:web#488'); D.runConfirm();
  assert.strictEqual(D.tk('PROJ-142').local.status,'done'); clean(html(),'end of flow');
});
T('confirm is blocked with an explanation when the tool is signed out, and offline sends nothing',()=>{
  D.SIM.jira(false); D.startActivation; const t=D.tk('PROJ-127'); t.drafts.push({id:'dx',kind:'comment',body:'hi',status:'draft'});
  D.ACT.postAsk({key:'PROJ-127',draft:'dx'}); let sh=S().ui.sheet; assert(sh.blocked&&html().includes('Cannot send')&&!html().includes('id="confirmBtn"'));
  D.SIM.jira(true); D.ACT.closeSheet(); D.SIM.toggle('offline',true); D.ACT.postAsk({key:'PROJ-127',draft:'dx'}); D.runConfirm();
  assert.strictEqual(t.drafts.find(d=>d.id==='dx').status,'draft','offline: nothing was sent'); assert(S().audit.some(a=>a.action==='jira.comment'&&a.outcome==='failure'));
});
T('hotfix activation asks for a baseline through the sheet',()=>{
  D.claim('PROJ-139'); D.startReview('PROJ-139'); D.markReviewed('PROJ-139'); D.ACT.activate({key:'PROJ-139'});
  assert.strictEqual(S().ui.sheet.kind,'baseline'); assert(html().includes('is a hotfix')); D.ACT.pickBaseline({}, {value:'production'}); D.ACT.activateWith({key:'PROJ-139'});
  assert.strictEqual(D.tk('PROJ-139').local.status,'active'); assert.strictEqual(S().ws.branches.worker,'master');
});
T('ambiguity is shown on the overview and blocks activation with a message',()=>{
  D.claim('PROJ-150'); D.startReview('PROJ-150'); D.markReviewed('PROJ-150'); go({name:'ticket',key:'PROJ-150',tab:'overview'}); assert(html().includes('2 branches match'));
  D.ACT.activate({key:'PROJ-150'}); assert.strictEqual(S().ui.sheet.kind,'errors'); D.ACT.closeSheet();
  D.ACT.choose({key:'PROJ-150',repo:'web',branch:'feature/PROJ-150-header'}); D.ACT.activate({key:'PROJ-150'}); assert.strictEqual(D.tk('PROJ-150').local.status,'active');
});
T('review: inline comment goes through the confirm sheet and appears as a thread',()=>{
  go({name:'ticket',key:'PROJ-142',tab:'review'}); D.ACT.inlineOpen({key:'PROJ-142',pr:'212',file:'src/redirect.rs',line:'n44'}); assert(html().includes('id="inl"'));
  D.TEXTINPUT('inl:0','Consider a test for a relative path with a query string.'); D.ACT.inlineAsk(); const sh=S().ui.sheet; assert(sh.kind==='confirm'&&sh.lead.includes('line 44')); D.runConfirm();
  assert(D.tk('PROJ-142').prs[0].threads.some(t=>t.author==='You'&&t.line==='n44')); assert(html().includes('Consider a test for a relative path'));
});
T('approve is disabled until signed off',()=>{
  go({name:'ticket',key:'PROJ-142',tab:'review'}); const h=html(); assert(/Approve…<\/button>/.test(h)&&h.includes('disabled')&&h.includes('unlocks when the ticket reaches a signed-off status'));
});
T('simulate panel renders and its switches work',()=>{
  S().ui.dev=true; assert(html().includes('Simulate')&&html().includes('uat conflict')); D.ACT.sim({k:'conflict'}); assert.strictEqual(S().sim.conflict,'web'); D.ACT.sim({k:'offline'}); assert(S().sim.offline);
});
T('command palette lists tickets',()=>{ D.ACT.palette(); const h=html(); assert(h.includes('PROJ-142')&&h.includes('pal-l')); D.TEXTINPUT('palq:0','header'); assert(html().includes('PROJ-150')&&!html().includes('PROJ-160  Update')); });
T('workspace stop warns about uncommitted work',()=>{ D.ACT.stopAsk(); const sh=S().ui.sheet; assert(sh.kind==='confirm'&&sh.risk==='low'&&sh.payload[0].includes('web')); D.runConfirm(); assert.strictEqual(S().ws.up,false); });
T('ticket facts are not shown twice, and appear in the overview when the right sidebar is hidden',()=>{
  go({name:'ticket',key:'PROJ-142',tab:'overview'}); const h=html();
  assert(h.includes('class="narrow-only"')&&h.includes('Linked issues'));
  assert((h.match(/Linked issues/g)||[]).length===2,'once in the right sidebar and once in the narrow-only copy: '+(h.match(/Linked issues/g)||[]).length);
});
console.log(n+' passed');
