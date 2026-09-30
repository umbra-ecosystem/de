const fs=require('fs'),vm=require('vm'),assert=require('assert');
const code=fs.readFileSync(__dirname+'/data.js','utf8')+'\n'+fs.readFileSync(__dirname+'/logic.js','utf8');
const ctx=vm.createContext({console,Date,Math,JSON,Object,Array,Set,Map,String,Number,Promise});
vm.runInContext(code,ctx); const D=ctx.__DE;
let n=0; const T=(name,fn)=>{ D.reset(); try{fn(); n++; console.log('ok  ',name);}catch(e){console.log('FAIL',name,'\n    ',e.message.split('\n')[0]); process.exitCode=1;} };
const tops=()=>D.visibleSuggestions().map(s=>s.rule+':'+(s.ticket||'-'));
const rules=(t)=>D.visibleSuggestions().filter(s=>s.ticket===t).map(s=>s.rule);
const checkAll=(k)=>{const t=D.tk(k); t.checklist.forEach((_,i)=>{ if(!t.checklist[i].done) D.toggleChecklist(k,i)});};

T('initial suggestions rank hotfix claim, mentions and reviews sensibly',()=>{
  const s=D.visibleSuggestions();
  assert(s.length>=5);
  assert(s.some(x=>x.rule==='start_review'&&x.ticket==='PROJ-142'),'start review 142');
  assert(s.some(x=>x.rule==='returned_mention'&&x.ticket==='PROJ-131'),'mention on returned 131');
  const claims=s.filter(x=>x.rule==='claim_new').map(x=>x.ticket);
  assert.strictEqual(claims[0],'PROJ-139','hotfix first among claims: '+claims);
  assert(s.find(x=>x.ticket==='PROJ-139'&&x.rule==='claim_new').priority>680-1,'hotfix boosted');
});

T('full lifecycle of PROJ-142: review, activate, test, integrate, deploy, announce, sign-off, approve',()=>{
  D.startReview('PROJ-142'); assert(rules('PROJ-142').includes('finish_review'));
  D.markReviewed('PROJ-142'); assert(rules('PROJ-142').includes('activate_reviewed'));
  const r=D.activate('PROJ-142'); assert(r.ok,JSON.stringify(r));
  const S=D.S; const t=D.tk('PROJ-142');
  assert.strictEqual(S.ws.branches['api-client'],'feature/PROJ-142-redirect');
  assert.strictEqual(S.ws.branches.web,'feature/PROJ-142-web');
  assert.strictEqual(S.ws.branches.worker,'develop'); assert.strictEqual(S.ws.branches.docs,'master');
  assert(t.act.overlay&&t.act.overlay.repo==='web','overlay on web');
  assert(t.act.records.find(x=>x.repo==='web').stash,'web was dirty so stashed');
  assert.strictEqual(S.ws.dirty.web,false);
  assert(!rules('PROJ-142').includes('integrate_ready'),'checklist incomplete');
  checkAll('PROJ-142'); assert(rules('PROJ-142').includes('integrate_ready'));
  const p=D.prepare('PROJ-142'); assert(p.ok); assert(t.prep.rows.every(x=>x.outcome==='ready'));
  assert(t.prep.rows.find(x=>x.repo==='web').overlaps.some(o=>o.key==='PROJ-127'),'overlap warning with PROJ-127');
  assert(rules('PROJ-142').includes('push_ready'));
  const pu=D.push('PROJ-142'); assert(pu.finalized,JSON.stringify(pu));
  assert.strictEqual(t.local.status,'integrated'); assert.strictEqual(t.act,null);
  assert.strictEqual(S.ws.branches.web,'wip/my-experiment','web restored'); assert.strictEqual(S.ws.dirty.web,true,'stash popped');
  assert(D.S.audit.some(a=>a.action==='overlay.revert'));
  assert(rules('PROJ-142').includes('deploy_waiting')); assert(!D.allDeployed(t));
  D.advance(9000); assert(D.allDeployed(t));
  assert(rules('PROJ-142').includes('compose_deploy_comment'));
  assert(D.composeDraft('PROJ-142').ok); assert(rules('PROJ-142').includes('post_deploy_comment'));
  const d=D.draftOf(t,'comment'); assert(d.body.includes('pipeline #')&&d.body.includes('api-client')&&d.body.includes('web'));
  assert(D.postComment('PROJ-142',d.id).ok); assert(!D.postComment('PROJ-142',d.id).ok,'no double post');
  assert(rules('PROJ-142').includes('transition_alpha')); assert(D.transition('PROJ-142').ok); assert.strictEqual(t.jira,'Alpha Testing');
  assert(!rules('PROJ-142').some(r=>['transition_alpha','post_deploy_comment','compose_deploy_comment'].includes(r)));
  assert(!rules('PROJ-142').includes('approve_prs'),'no approval before sign-off');
  D.SIM.signOff('PROJ-142'); const ap=D.visibleSuggestions().filter(s=>s.rule==='approve_prs'&&s.ticket==='PROJ-142'); assert.strictEqual(ap.length,2);
  D.approve('PROJ-142','api-client',212); assert.strictEqual(t.local.status,'integrated'); D.approve('PROJ-142','web',488); assert.strictEqual(t.local.status,'done');
});

T('a second ticket cannot be activated while one is active, and nothing moves',()=>{
  D.startReview('PROJ-142'); D.markReviewed('PROJ-142'); D.activate('PROJ-142');
  D.claim('PROJ-160'); D.startReview('PROJ-160'); D.markReviewed('PROJ-160');
  const before=JSON.stringify(D.S.ws.branches); const r=D.activate('PROJ-160'); assert(!r.ok); assert.strictEqual(JSON.stringify(D.S.ws.branches),before);
});

T('ambiguous branches block activation until one is chosen',()=>{
  D.claim('PROJ-150'); D.startReview('PROJ-150'); D.markReviewed('PROJ-150');
  const r=D.activate('PROJ-150'); assert(!r.ok&&r.errors[0].includes('web')); assert.strictEqual(D.S.ws.branches.web,'wip/my-experiment');
  D.setLink('PROJ-150','web','feature/PROJ-150-header'); assert(D.activate('PROJ-150').ok);
  assert.strictEqual(D.tk('PROJ-150').act.overlay,null,'no overlay: api-client untouched');
});

T('hotfix needs a baseline and uses production branches when chosen',()=>{
  D.claim('PROJ-139'); D.startReview('PROJ-139'); D.markReviewed('PROJ-139');
  assert(D.isHotfix(D.tk('PROJ-139')));
  const r=D.activate('PROJ-139'); assert(r.needBaseline);
  assert(D.activate('PROJ-139','production').ok);
  assert.strictEqual(D.S.ws.branches.worker,'master'); assert.strictEqual(D.S.ws.branches.docs,'master');
  assert.strictEqual(D.S.ws.branches['api-client'],'hotfix/PROJ-139-idempotency');
});

T('deactivate restores everything and parks',()=>{
  D.startReview('PROJ-142'); D.markReviewed('PROJ-142'); const b=JSON.stringify(D.S.ws.branches); D.activate('PROJ-142');
  const r=D.deactivate('PROJ-142','parked'); assert(r.ok); assert.strictEqual(JSON.stringify(D.S.ws.branches),b);
  assert.strictEqual(D.tk('PROJ-142').local.status,'parked'); assert.strictEqual(D.S.ws.dirty.web,true);
  assert(rules('PROJ-142').includes('activate_reviewed'));
});

function ready(k){ D.startReview(k); D.markReviewed(k); assert(D.activate(k).ok); checkAll(k); }
T('a merge conflict blocks the push and nothing is recorded',()=>{
  ready('PROJ-142'); D.SIM.toggle('conflict','web'); const p=D.prepare('PROJ-142'); assert(p.ok);
  assert(D.tk('PROJ-142').prep.rows.find(r=>r.repo==='web').outcome==='conflict'); assert(!D.push('PROJ-142').ok);
  assert.strictEqual(D.tk('PROJ-142').merges.length,0); assert(rules('PROJ-142').includes('integration_blocked'));
});
T('an overlay leak blocks integration',()=>{
  ready('PROJ-142'); D.SIM.toggle('leak','api-client'); D.prepare('PROJ-142');
  assert(D.tk('PROJ-142').prep.rows.find(r=>r.repo==='api-client').outcome==='blocked'); assert(!D.push('PROJ-142').ok);
});
T('uat moving between prepare and push rejects and requires preparing again',()=>{
  ready('PROJ-142'); D.prepare('PROJ-142'); D.SIM.toggle('uatMoved',true); const r=D.push('PROJ-142');
  assert(r.results[0].result==='Rejected'); assert.strictEqual(D.tk('PROJ-142').merges.length,0); assert.strictEqual(D.tk('PROJ-142').prep,null);
  assert.strictEqual(D.tk('PROJ-142').local.status,'active');
  D.prepare('PROJ-142'); assert(D.push('PROJ-142').finalized);
});
T('a failed deploy offers a re-run, and the re-run succeeds',()=>{
  ready('PROJ-142'); D.SIM.toggle('failDeploy','web'); D.prepare('PROJ-142'); D.push('PROJ-142'); D.advance(9000);
  assert.strictEqual(D.tk('PROJ-142').deploy.web.state,'failed'); assert(rules('PROJ-142').includes('deploy_failed'));
  assert(!D.tk('PROJ-142').drafts.length); D.rerun('PROJ-142','web'); D.advance(9000);
  assert.strictEqual(D.tk('PROJ-142').deploy.web.state,'deployed'); assert(D.allDeployed(D.tk('PROJ-142')));
});
T('signed-out bkt replaces bkt actions with an explanation, never hides them silently',()=>{
  ready('PROJ-142'); D.SIM.toggle('failDeploy','web'); D.prepare('PROJ-142'); D.push('PROJ-142'); D.advance(9000);
  D.SIM.bkt(false); const s=D.visibleSuggestions().filter(x=>x.ticket==='PROJ-142'); assert(s.some(x=>x.rule==='adapter_unavailable'&&x.title.includes('bkt')));
  assert(!s.some(x=>x.rule==='deploy_failed'));
});
T('dismissal hides a suggestion until its facts change',()=>{
  const s=D.visibleSuggestions().find(x=>x.rule==='returned_mention'&&x.ticket==='PROJ-131'); D.dismiss(s.id);
  assert(!D.visibleSuggestions().some(x=>x.id===s.id));
  D.SIM.mention('PROJ-131'); const again=D.visibleSuggestions().find(x=>x.ticket==='PROJ-131'&&x.rule==='returned_mention'); assert(again&&again.state==='resurfaced');
});
T('opening a ticket marks its comments seen and clears the mention',()=>{
  assert(D.unseenMentions(D.tk('PROJ-131')).length===1); D.markSeen('PROJ-131'); assert(!rules('PROJ-131').includes('returned_mention'));
});
T('snooze hides then returns after the time passes',()=>{
  const s=D.visibleSuggestions().find(x=>x.rule==='claim_new'); D.snooze(s.id,2);
  assert(!D.visibleSuggestions().some(x=>x.id===s.id)); D.advance(9000); assert(D.visibleSuggestions().some(x=>x.id===s.id));
});
T('sync applies scripted arrivals; offline keeps the cache and reports it',()=>{
  const before=D.S.tickets.length; const r=D.syncNow(); assert(r.ok); assert.strictEqual(D.S.tickets.length,before+1);
  D.SIM.toggle('offline',true); const r2=D.syncNow(); assert(!r2.ok); assert.strictEqual(D.S.tickets.length,before+1); assert(D.S.sync.lastError);
});
T('new commits after integration ask for a re-merge; a returned ticket back in review is claimed again',()=>{
  ready('PROJ-142'); D.prepare('PROJ-142'); D.push('PROJ-142'); D.advance(9000);
  D.SIM.newCommits('PROJ-142'); assert(rules('PROJ-142').includes('remerge_needed'));
  D.SIM.returned('PROJ-127'); assert(D.tk('PROJ-127').jira==='Returned');
  D.SIM.backToReview('PROJ-127'); D.tk('PROJ-127').drafts[0].status='posted'; assert(rules('PROJ-127').includes('returned_to_review'));
  D.reclaim('PROJ-127'); assert.strictEqual(D.tk('PROJ-127').local.status,'claimed'); assert.strictEqual(D.tk('PROJ-127').merges.length,0);
});
T('a waiting hotfix suggests parking the active ticket',()=>{
  ready('PROJ-142'); D.claim('PROJ-139'); D.startReview('PROJ-139'); D.markReviewed('PROJ-139');
  const s=D.visibleSuggestions().find(x=>x.rule==='park_active'); assert(s&&s.ticket==='PROJ-142');
});
T('a returned ticket gets no announce-flow suggestions, only mentions',()=>{
  const r=D.visibleSuggestions().filter(x=>x.ticket==='PROJ-131').map(x=>x.rule);
  assert(r.includes('returned_mention')); assert(!r.some(x=>['compose_deploy_comment','post_deploy_comment','transition_alpha','deploy_waiting'].includes(x)),'got '+r);
});
console.log(n+' passed');
