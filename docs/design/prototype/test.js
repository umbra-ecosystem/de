const fs=require('fs'),vm=require('vm'),assert=require('assert');
const code=fs.readFileSync(__dirname+'/data.js','utf8')+'\n'+fs.readFileSync(__dirname+'/logic.js','utf8');
const ctx=vm.createContext({console,Date,Math,JSON,Object,Array,Set,Map,String,Number,Promise});
vm.runInContext(code,ctx); const D=ctx.__DE;
let n=0; const T=(name,fn)=>{ D.reset(); try{fn(); n++; console.log('ok  ',name);}catch(e){console.log('FAIL',name,'\n    ',e.message.split('\n')[0]); process.exitCode=1;} };
const tops=()=>D.visibleSuggestions().map(s=>s.rule+':'+(s.ticket||'-'));
const rules=(t)=>D.visibleSuggestions().filter(s=>s.ticket===t).map(s=>s.rule);
const tickPre=(k)=>D.prepushItems(D.tk(k)).forEach(x=>{ if(!x.done) D.togglePre(k,x.repo,x.i); });
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
  assert(rules('PROJ-142').includes('prepush_checks')&&!rules('PROJ-142').includes('push_ready'),'push waits for the repo checks'); assert(!D.push('PROJ-142').ok,'push refused until checks are ticked');
  tickPre('PROJ-142'); assert(rules('PROJ-142').includes('push_ready')&&!rules('PROJ-142').includes('prepush_checks'));
  const pu=(tickPre('PROJ-142'),D.push('PROJ-142')); assert(pu.finalized,JSON.stringify(pu));
  assert.strictEqual(t.local.status,'integrated'); assert.strictEqual(t.act,null);
  assert.strictEqual(S.ws.branches.web,'wip/my-experiment','web restored'); assert.strictEqual(S.ws.dirty.web,true,'stash popped');
  assert(D.S.audit.some(a=>a.action==='overlay.revert'));
  assert(rules('PROJ-142').includes('deploy_waiting')); assert(!D.allDeployed(t));
  D.advance(9000); assert(D.allDeployed(t));
  assert(rules('PROJ-142').includes('compose_deploy_comment'));
  D.postFreeComment('PROJ-142','Checked the retry path on staging.'); assert(D.composeDraft('PROJ-142').ok); assert(rules('PROJ-142').includes('post_deploy_comment'));
  const d=D.draftOf(t,'comment'); assert(d.body.includes('Tested locally:')&&d.body.includes(t.checklist[0].text)&&d.body.includes('Comments while testing:')&&d.body.includes('Checked the retry path'),'checklist and testing comments are in the draft'); assert(d.body.includes('run #')&&d.body.includes('api-client')&&d.body.includes('web'));
  assert(D.postComment('PROJ-142',d.id).ok); assert(!D.postComment('PROJ-142',d.id).ok,'no double post');
  assert(rules('PROJ-142').includes('transition_alpha')); assert(D.transition('PROJ-142').ok); assert.strictEqual(t.jira,'Alpha Testing');
  assert(!rules('PROJ-142').some(r=>['transition_alpha','post_deploy_comment','compose_deploy_comment'].includes(r)));
  assert(!rules('PROJ-142').includes('approve_prs'),'no approval before sign-off');
  D.SIM.signOff('PROJ-142'); const ap=D.visibleSuggestions().filter(s=>s.rule==='approve_prs'&&s.ticket==='PROJ-142'); assert.strictEqual(ap.length,1); assert(ap[0].id==='PROJ-142:approve:all','two pending PRs are one batch approval');
  D.approve('PROJ-142','api-client',212); assert.strictEqual(t.local.status,'integrated'); assert(D.visibleSuggestions().some(s=>s.id==='PROJ-142:approve:api-client#212'||s.id==='PROJ-142:approve:web#488'),'one left: single approval again'); D.approve('PROJ-142','web',488); assert.strictEqual(t.local.status,'done');
});

T('a second ticket cannot be activated while one is active, and nothing moves',()=>{
  D.startReview('PROJ-142'); D.markReviewed('PROJ-142'); D.activate('PROJ-142');
  (D.unclaim('PROJ-142'),D.claim('PROJ-160')); D.startReview('PROJ-160'); D.markReviewed('PROJ-160');
  const before=JSON.stringify(D.S.ws.branches); const r=D.activate('PROJ-160'); assert(!r.ok); assert.strictEqual(JSON.stringify(D.S.ws.branches),before);
});

T('ambiguous branches block activation until one is chosen',()=>{
  (D.unclaim('PROJ-142'),D.claim('PROJ-150')); D.startReview('PROJ-150'); D.markReviewed('PROJ-150');
  const r=D.activate('PROJ-150'); assert(!r.ok&&r.errors[0].includes('web')); assert.strictEqual(D.S.ws.branches.web,'wip/my-experiment');
  D.setLink('PROJ-150','web','feature/PROJ-150-header'); assert(D.activate('PROJ-150').errors.some(e=>e.includes('unresolved')),'open comment blocks activation'); D.SIM.resolveThreads('PROJ-150'); assert(D.activate('PROJ-150').ok);
  assert.strictEqual(D.tk('PROJ-150').act.overlay,null,'no overlay: api-client untouched');
});

T('hotfix needs a baseline and uses production branches when chosen',()=>{
  (D.unclaim('PROJ-142'),D.claim('PROJ-139')); D.startReview('PROJ-139'); D.markReviewed('PROJ-139');
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
  assert(D.tk('PROJ-142').prep.rows.find(r=>r.repo==='web').outcome==='conflict'); assert(!(tickPre('PROJ-142'),D.push('PROJ-142')).ok);
  assert.strictEqual(D.tk('PROJ-142').merges.length,0); assert(rules('PROJ-142').includes('integration_blocked'));
});
T('an overlay leak blocks integration',()=>{
  ready('PROJ-142'); D.SIM.toggle('leak','api-client'); D.prepare('PROJ-142');
  assert(D.tk('PROJ-142').prep.rows.find(r=>r.repo==='api-client').outcome==='blocked'); assert(!(tickPre('PROJ-142'),D.push('PROJ-142')).ok);
});
T('uat moving between prepare and push rejects and requires preparing again',()=>{
  ready('PROJ-142'); D.prepare('PROJ-142'); D.SIM.toggle('uatMoved',true); const r=(tickPre('PROJ-142'),D.push('PROJ-142'));
  assert(r.results[0].result==='Rejected'); assert.strictEqual(D.tk('PROJ-142').merges.length,0); assert.strictEqual(D.tk('PROJ-142').prep,null);
  assert.strictEqual(D.tk('PROJ-142').local.status,'active');
  D.prepare('PROJ-142'); assert((tickPre('PROJ-142'),D.push('PROJ-142')).finalized);
});
T('a failed deploy offers a re-run, and the re-run succeeds',()=>{
  ready('PROJ-142'); D.SIM.toggle('failDeploy','web'); D.prepare('PROJ-142'); (tickPre('PROJ-142'),D.push('PROJ-142')); D.advance(9000);
  assert.strictEqual(D.tk('PROJ-142').deploy.web.state,'failed'); assert(rules('PROJ-142').includes('deploy_failed'));
  assert(!D.tk('PROJ-142').drafts.length); D.rerun('PROJ-142','web'); D.advance(9000);
  assert.strictEqual(D.tk('PROJ-142').deploy.web.state,'deployed'); assert(D.allDeployed(D.tk('PROJ-142')));
});
T('signed-out gh replaces gh actions with an explanation, never hides them silently',()=>{
  ready('PROJ-142'); D.SIM.toggle('failDeploy','web'); D.prepare('PROJ-142'); (tickPre('PROJ-142'),D.push('PROJ-142')); D.advance(9000);
  D.SIM.gh(false); const s=D.visibleSuggestions().filter(x=>x.ticket==='PROJ-142'); assert(s.some(x=>x.rule==='adapter_unavailable'&&x.title.includes('gh')));
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
  ready('PROJ-142'); D.prepare('PROJ-142'); (tickPre('PROJ-142'),D.push('PROJ-142')); D.advance(9000);
  D.SIM.newCommits('PROJ-142'); assert(rules('PROJ-142').includes('remerge_needed'));
  D.SIM.returned('PROJ-127'); assert(D.tk('PROJ-127').jira==='Returned');
  D.SIM.backToReview('PROJ-127'); D.tk('PROJ-127').drafts[0].status='posted'; assert(rules('PROJ-127').includes('returned_to_review'));
  D.reclaim('PROJ-127'); assert.strictEqual(D.tk('PROJ-127').local.status,'claimed'); assert.strictEqual(D.tk('PROJ-127').merges.length,0);
});
T('a waiting hotfix suggests parking the active ticket',()=>{
  ready('PROJ-142'); const h=D.tk('PROJ-139'); h.local={status:'parked'}; D.markReviewed('PROJ-139');
  const s=D.visibleSuggestions().find(x=>x.rule==='park_active'); assert(s&&s.ticket==='PROJ-142');
});
T('a returned ticket gets no announce-flow suggestions, only mentions',()=>{
  const r=D.visibleSuggestions().filter(x=>x.ticket==='PROJ-131').map(x=>x.rule);
  assert(r.includes('returned_mention')); assert(!r.some(x=>['compose_deploy_comment','post_deploy_comment','transition_alpha','deploy_waiting'].includes(x)),'got '+r);
});
T('uat pre-check finds conflicts and overlaps before any integration',()=>{
  D.claim('PROJ-142'); const k=D.tk('PROJ-142');
  assert.strictEqual(D.uatConflicts(k).length,0); assert(D.uatOverlaps(k).some(r=>r.repo==='web'),'overlap with PROJ-127 on uat');
  D.SIM.toggle('conflict','web'); assert(D.uatConflicts(k).some(r=>r.repo==='web'));
  assert(rules('PROJ-142').includes('uat_conflict')&&D.attention().some(s=>s.rule==='uat_conflict'),'conflict is urgent');
  D.SIM.toggle('conflict',null); assert(!rules('PROJ-142').includes('uat_conflict'));
});
T('hunks split at gaps of context, split view pairs deletes with adds',()=>{
  const f=D.tk('PROJ-142').prs[0].files[0]; const h=D.hunksOf(f);
  assert.strictEqual(h.length,2); assert.strictEqual(h[0].a,0); assert.strictEqual(h[h.length-1].b,f.lines.length); assert.strictEqual(h[0].b,h[1].a);
  const rows=D.splitRows(f.lines); assert(rows.some(r=>r.l&&r.l.k==='del'&&r.r&&r.r.k==='add'),'the del sits opposite an add');
});
T('a failed deploy keeps its log tail, and uat moving after a push is flagged',()=>{
  ready('PROJ-142'); D.SIM.toggle('failDeploy','web'); D.prepare('PROJ-142'); (tickPre('PROJ-142'),D.push('PROJ-142')); D.advance(9000);
  const d=D.tk('PROJ-142').deploy.web; assert.strictEqual(d.state,'failed'); assert(d.log.some(x=>x.includes('health check')));
  D.rerun('PROJ-142','web'); assert.strictEqual(d.log,null);
  D.SIM.uatAfterPush('PROJ-142'); assert(rules('PROJ-142').includes('uat_moved'));
});
T('a ticket without a pull request cannot be reviewed; it waits, then prepares a return comment',()=>{
  const k=D.tk('PROJ-163'); assert(D.hasPrGap(k)); assert(!D.startReview('PROJ-163').ok,'no review without a PR');
  (D.unclaim('PROJ-142'),D.claim('PROJ-163')); assert(k.prWait&&k.prWait.mins===30,'claiming starts the wait');
  assert(!D.startReview('PROJ-163').ok&&!D.markReviewed('PROJ-163').ok);
  assert(rules('PROJ-163').includes('pr_waiting')&&!rules('PROJ-163').includes('start_review')&&!rules('PROJ-163').includes('pr_missing_return'));
  D.SIM.skipMin(29); assert(rules('PROJ-163').includes('pr_waiting'),'still waiting at 29 min');
  D.SIM.skipMin(2); assert(D.prWaitExpired(k)); assert(rules('PROJ-163').includes('pr_missing_return')&&D.attention().some(s=>s.rule==='pr_missing_return'));
  D.extendPrWait('PROJ-163',30); assert(!D.prWaitExpired(k)&&rules('PROJ-163').includes('pr_waiting'),'waiting longer restarts the clock');
  D.SIM.skipMin(31); const body=D.returnBody(k); assert(body.includes('no pull request')&&body.includes('feature/PROJ-163-export')&&body.includes('Waited 30 min'),body);
  const before=k.comments.length; assert(D.returnMissingPr('PROJ-163').ok); assert.strictEqual(k.jira,'Returned'); assert.strictEqual(k.comments.length,before+1); assert.strictEqual(k.local,null);
});
T('the wait ends by itself when the pull request shows up',()=>{
  (D.unclaim('PROJ-142'),D.claim('PROJ-163')); D.SIM.skipMin(31); assert(rules('PROJ-163').includes('pr_missing_return'));
  D.SIM.prArrives('PROJ-163'); assert(!D.hasPrGap(D.tk('PROJ-163'))); assert(!rules('PROJ-163').some(r=>r==='pr_missing_return'||r==='pr_waiting')); assert(rules('PROJ-163').includes('start_review')); assert(D.startReview('PROJ-163').ok);
  assert(!D.returnMissingPr('PROJ-163').ok,'nothing to return once a PR exists');
});
T('unresolved review comments block activation, wait, then prepare a return comment; proceeding anyway is a recorded warning',()=>{
  const k=D.tk('PROJ-150'); (D.unclaim('PROJ-142'),D.claim('PROJ-150')); D.startReview('PROJ-150'); D.setLink('PROJ-150','web','feature/PROJ-150-header');
  assert.strictEqual(D.blockingThreads(k).length,1); assert.strictEqual(k.thWait,undefined,'the wait starts when the review is finished'); assert(!rules('PROJ-150').includes('threads_waiting'));
  D.markReviewed('PROJ-150'); assert.strictEqual(k.thWait.mins,30); assert(rules('PROJ-150').includes('threads_waiting')&&!rules('PROJ-150').includes('activate_reviewed'));
  assert(D.activate('PROJ-150').errors.some(e=>e.includes('unresolved')));
  D.SIM.skipMin(31); assert(D.thWaitExpired(k)); const s=D.visibleSuggestions().find(x=>x.rule==='threads_return'); assert(s&&s.alt&&s.alt.do==='acceptThreadsAsk'&&D.attention().some(x=>x.id===s.id));
  const body=D.returnThreadsBody(k); assert(body.includes('src/header.css:12 (Marta Lind)')&&body.includes('Waited 30 min'),body);
  D.acceptThreads('PROJ-150'); assert.strictEqual(D.blockingThreads(k).length,0); assert.strictEqual(D.acceptedThreads(k).length,1); assert(D.activate('PROJ-150').ok,'proceeding anyway unblocks activation');
});
T('unresolved comments that get resolved release the ticket; returning sends the list',()=>{
  const k=D.tk('PROJ-150'); (D.unclaim('PROJ-142'),D.claim('PROJ-150')); D.startReview('PROJ-150'); D.markReviewed('PROJ-150'); D.SIM.skipMin(31);
  assert(D.returnThreads('PROJ-150').ok); assert.strictEqual(k.jira,'Returned'); assert.strictEqual(k.local,null); assert(k.comments.slice(-1)[0].body.some(b=>b.x.includes('still unresolved')));
  D.reset(); (D.unclaim('PROJ-142'),D.claim('PROJ-150')); D.startReview('PROJ-150'); D.markReviewed('PROJ-150'); D.SIM.resolveThreads('PROJ-150'); assert.strictEqual(D.blockingThreads(D.tk('PROJ-150')).length,0); assert(!D.returnThreads('PROJ-150').ok);
});
T('a uat conflict has a prepared comment, optionally returning the ticket',()=>{
  const k=D.tk('PROJ-142'); D.claim('PROJ-142'); D.SIM.toggle('conflict','web');
  const s=D.visibleSuggestions().find(x=>x.rule==='uat_conflict'); assert(s&&s.act.do==='conflictAsk'&&s.level==='external');
  const body=D.conflictBody(k); assert(body.includes('PROJ-127')&&body.includes('src/session.rs')&&body.includes('rebase'),body);
  const n=k.comments.length; assert(D.sendConflict('PROJ-142',false).ok); assert.strictEqual(k.comments.length,n+1); assert.strictEqual(k.jira,'In Review'); assert(D.conflictSent(k)); assert(!rules('PROJ-142').includes('uat_conflict'),'no nagging after it was sent');
  assert(D.sendConflict('PROJ-142',true).ok); assert.strictEqual(k.jira,'Returned'); assert.strictEqual(k.local,null);
});
T('returning an active ticket for a conflict parks it first so repos are restored',()=>{
  ready('PROJ-142'); assert.strictEqual(D.tk('PROJ-142').local.status,'active'); D.SIM.toggle('conflict','web');
  assert(D.sendConflict('PROJ-142',true).ok); assert.strictEqual(D.tk('PROJ-142').act,null); assert.strictEqual(D.tk('PROJ-142').local,null); assert.strictEqual(D.S.ws.branches.web,'wip/my-experiment');
});
T('only one ticket can be in hand: claimed, in review or active',()=>{
  assert.strictEqual(D.tk('PROJ-142').local.status,'claimed');
  const r=D.claim('PROJ-150'); assert(!r.ok&&r.blocker==='PROJ-142'); assert.strictEqual(D.tk('PROJ-150').local,null);
  const sr=D.startReview('PROJ-150'); assert(!sr.ok&&sr.blocker==='PROJ-142','review cannot start on an unclaimed ticket either');
  const s=D.visibleSuggestions().filter(x=>x.rule==='claim_new'); assert(!s.some(x=>x.ticket==='PROJ-150'),'no claim suggestion for a normal ticket'); assert(s.some(x=>x.ticket==='PROJ-139'),'a hotfix can still be offered');
  assert(!D.claim('PROJ-139').ok,'the hotfix is blocked too, but can be swapped in');
  assert(D.parkInHand('PROJ-142').ok); assert.strictEqual(D.tk('PROJ-142').local.status,'parked'); assert(D.claim('PROJ-139').ok,'parked tickets do not count');
  assert(D.claim('PROJ-150').blocker==='PROJ-139');
});
T('parking the active ticket to take another restores the repos',()=>{
  ready('PROJ-142'); assert(D.claim('PROJ-150').blocker==='PROJ-142');
  assert(D.parkInHand('PROJ-142').ok); assert.strictEqual(D.tk('PROJ-142').act,null); assert.strictEqual(D.S.ws.branches.web,'wip/my-experiment'); assert(D.claim('PROJ-150').ok);
});
T('tickets awaiting alpha do not count as in hand',()=>{
  D.unclaim('PROJ-142'); assert.strictEqual(D.inHand(),undefined); assert(D.claim('PROJ-150').ok);
});
T('a repo held by another process blocks activation, all or nothing, and changes nothing',()=>{
  D.startReview('PROJ-142'); D.markReviewed('PROJ-142'); const before=JSON.stringify(D.S.ws.branches);
  D.SIM.extLock('docs'); const r=D.activate('PROJ-142'); assert(!r.ok&&r.busy&&r.busy.repo==='docs','a repo the ticket only baselines still counts'); assert(r.errors[0].includes('de CLI')&&r.errors[0].includes('Nothing was changed'));
  assert.strictEqual(JSON.stringify(D.S.ws.branches),before); assert.strictEqual(D.tk('PROJ-142').local.status,'reviewing'); assert(D.S.audit.some(a=>a.action==='lock.denied'&&a.repo==='docs'),'denials are audited');
  D.SIM.extLock('docs'); assert(D.activate('PROJ-142').ok,'released');
});
T('locks are taken in a fixed order, so the first blocking repo is reported the same way every time',()=>{
  D.startReview('PROJ-142'); D.markReviewed('PROJ-142'); D.SIM.extLock('worker'); D.SIM.extLock('api-client');
  assert.strictEqual(D.activate('PROJ-142').busy.repo,'api-client');
});
T('a leftover git lock is stale: shown as attention, never removed silently',()=>{
  D.startReview('PROJ-142'); D.markReviewed('PROJ-142'); D.SIM.staleLock('web');
  const r=D.activate('PROJ-142'); assert(!r.ok&&r.errors[0].includes('leftover .git/index.lock')); const s=D.visibleSuggestions().find(x=>x.rule==='stale_lock'); assert(s&&D.attention().some(x=>x.id===s.id));
  D.SIM.extLock('worker'); assert(!D.breakLock('worker').ok,'a live holder is not stale');
  assert(D.breakLock('web').ok); assert(!D.visibleSuggestions().some(x=>x.rule==='stale_lock')); D.SIM.extLock('worker'); assert(D.activate('PROJ-142').ok);
});
T('with holds on, an operation locks its repos until it finishes, and sync yields to it',()=>{
  D.SIM.toggle('holdLocks',true); D.startReview('PROJ-142'); D.markReviewed('PROJ-142');
  assert(D.activate('PROJ-142').ok); const p=D.deactivate('PROJ-142','parked'); assert(!p.ok&&p.errors[0].includes('de is activating PROJ-142'),p.errors&&p.errors[0]);
  D.syncStart(); const f=D.syncFinish(); assert(f.report.some(x=>x.src==='Git fetch'&&x.text.includes('Fetches as soon as it is free')),'sync queues busy repos and says so');
  D.advance(4000); assert(D.deactivate('PROJ-142','parked').ok,'released once the operation is over');
});
T('sync holds the repos while it fetches, so an activation waits for it',()=>{
  D.SIM.toggle('holdLocks',true); D.startReview('PROJ-142'); D.markReviewed('PROJ-142'); D.syncStart();
  const r=D.activate('PROJ-142'); assert(!r.ok&&r.errors[0].includes('background sync is fetching')); D.syncFinish(); assert(D.activate('PROJ-142').ok);
});
T('returning an active ticket does nothing if its repos are busy',()=>{
  ready('PROJ-142'); D.SIM.toggle('conflict','web'); D.SIM.extLock('web'); const n=D.tk('PROJ-142').comments.length;
  const r=D.sendConflict('PROJ-142',true); assert(!r.ok&&r.busy); assert.strictEqual(D.tk('PROJ-142').jira,'In Review'); assert.strictEqual(D.tk('PROJ-142').comments.length,n,'no comment without the move');
});
T('sync queues a busy repo, fetches it the moment it is free, and never skips silently',()=>{
  D.SIM.extLock('web'); D.syncStart(); const f=D.syncFinish(); assert(D.S.pending.web,'queued'); assert(f.report.some(x=>x.text.includes('web is busy')&&x.text.includes('gives up after 30 s')));
  assert.strictEqual(D.S.sync.lastError,null,'waiting is not a failure yet'); D.advance(5000); assert(D.S.pending.web,'still busy, still waiting');
  D.SIM.extLock('web'); const at=D.S.ms; D.advance(500); assert(!D.S.pending.web&&D.S.repoFetch.web>=at,'fetched as soon as it was free'); assert(!D.S.staleRepos.web);
});
T('after the timeout the repo is marked not refreshed, the report fails, and the next sync clears it',()=>{
  D.SIM.extLock('web'); D.syncStart(); D.syncFinish(); D.advance(31000);
  assert(D.S.staleRepos.web&&!D.S.pending.web); assert.strictEqual(D.S.sync.lastError,'partial'); assert(D.S.sync.report.some(x=>!x.ok&&x.text.includes('web not refreshed')));
  assert(D.visibleSuggestions().some(x=>x.rule==='repo_stale'&&x.act.do==='sync')); assert(D.S.audit.some(a=>a.action==='sync.fetch_timeout'));
  D.SIM.extLock('web'); D.syncStart(); D.syncFinish(); assert(!D.S.staleRepos.web,'a successful fetch clears it'); assert(!D.visibleSuggestions().some(x=>x.rule==='repo_stale'));
});
T('preparing an integration fetches under its own lock, so it does not trust a stale sync',()=>{
  ready('PROJ-142'); D.S.staleRepos.web={since:0,by:'x'}; D.prepare('PROJ-142'); assert(!D.S.staleRepos.web);
});
console.log(n+' passed');
