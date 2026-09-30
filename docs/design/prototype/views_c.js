'use strict';
/* ============================ sheets, actions, boot ============================ */

const IS_BROWSER = typeof document !== 'undefined';

/* ---------------- confirm sheet (the gateway preview) ---------------- */
function openConfirm(o) {
  const blocked = o.needs && !S.providers[o.needs === 'acli' ? 'jira' : 'bkt'].ready ? (o.needs === 'acli' ? 'acli' : 'bkt') + ' is signed out. Nothing can be sent until you log in.' : null;
  S.ui.sheet = Object.assign({ kind: 'confirm', typed: '', risk: 'medium', blocked }, o);
}
function closeSheet() { S.ui.sheet = null; }

function askPush(key) {
  const t = tk(key); if (!t.prep || !prepPushable(t)) { toast('Prepare the integration first.', 'warn'); return; }
  const rows = t.prep.rows.filter((r) => r.outcome === 'ready');
  openConfirm({
    risk: 'high', title: 'Push to uat', ticket: key, need: key, auditName: 'git.push_uat',
    lead: 'This will run, exactly:', payload: rows.map((r) => r.repo.padEnd(11) + ' git push origin ' + r.merge + ':refs/heads/uat'),
    details: rows.map((r) => ({ a: r.repo, b: 'uat ' + r.uatBefore + ' → ' + r.merge, c: '+' + r.commits + ' commits · ' + r.files + ' files' })),
    checks: ['overlay guard ✓ (by SHA)', 'no overlay in worktree ✓', 'no force'], warn: 'Pushing deploys to alpha. Nothing has been sent yet.',
    confirmLabel: 'Push to uat', exec: () => { const r = push(key);
      if (!r.ok) return { toast: [r.error, 'bad'] };
      return { sheet: { kind: 'result', title: 'Push to uat', ticket: key, rows: r.results, finalized: r.finalized } }; },
  });
}
function askPostDraft(key, draftId) {
  const t = tk(key); const d = t.drafts.find((x) => x.id === draftId);
  openConfirm({ risk: 'medium', title: 'Post a comment to Jira', ticket: key, needs: 'acli', auditName: 'jira.comment', lead: 'This exact text will be posted as you:', payload: d.body.split('\n'), warn: 'Posted comments cannot be recalled from here.', confirmLabel: 'Post comment',
    exec: () => { const r = postComment(key, draftId); return { toast: [r.ok ? 'Comment posted to ' + key : r.error, r.ok ? 'ok' : 'bad'] }; } });
}
function askTransition(key) {
  const t = tk(key);
  openConfirm({ risk: 'medium', title: 'Move the ticket in Jira', ticket: key, needs: 'acli', auditName: 'jira.transition', lead: 'This will transition:', payload: [key + '   ' + t.jira + ' → ' + JIRA.alpha], warn: 'Others watch this status.', confirmLabel: 'Transition',
    exec: () => { const r = transition(key); return { toast: [r.ok ? key + ' moved to ' + JIRA.alpha : r.error, r.ok ? 'ok' : 'bad'] }; } });
}
function askApprove(key, repo, pr) {
  openConfirm({ risk: 'medium', title: 'Approve a pull request', ticket: key, needs: 'bkt', auditName: 'bitbucket.pr_approve', lead: 'This will approve:', payload: [REPOS[repo].host + ' #' + pr], warn: 'Approval is the last gate. It only unlocks after sign-off.', confirmLabel: 'Approve',
    exec: () => { approve(key, repo, pr); return { toast: ['Approved ' + repo + ' #' + pr, 'ok'] }; } });
}
function askRerun(key, repo) {
  const d = tk(key).deploy[repo];
  openConfirm({ risk: 'medium', title: 'Re-run a pipeline', ticket: key, needs: 'bkt', auditName: 'bitbucket.pipeline_rerun', lead: 'This will re-run:', payload: [REPOS[repo].host + ' pipeline #' + d.run + ' (deploy to alpha)'], warn: 'Re-running redeploys to alpha.', confirmLabel: 'Re-run',
    exec: () => { rerun(key, repo); return { toast: ['Re-running ' + repo, 'info'] }; } });
}
function askComment(key) {
  const text = (S.ui.drafts['cmt:' + key] || '').trim(); if (!text) { toast('Write a comment first.', 'warn'); return; }
  openConfirm({ risk: 'medium', title: 'Post a comment to Jira', ticket: key, needs: 'acli', auditName: 'jira.comment', lead: 'This exact text will be posted as you:', payload: text.split('\n'), confirmLabel: 'Post comment',
    exec: () => { postFreeComment(key, text); S.ui.drafts['cmt:' + key] = ''; return { toast: ['Comment posted to ' + key, 'ok'] }; } });
}
function askInline(key) {
  const inl = S.ui.inline; const text = (S.ui.drafts['inl:0'] || '').trim(); if (!inl || !text) { toast('Write a comment first.', 'warn'); return; }
  const pr = tk(key).prs.find((p) => p.id === inl.pr);
  openConfirm({ risk: 'medium', title: 'Post an inline PR comment', ticket: key, needs: 'bkt', auditName: 'bitbucket.pr_comment', lead: REPOS[pr.repo].host + ' #' + pr.id + ' · ' + inl.file + ' line ' + inl.line.slice(1) + ' (' + (inl.line[0] === 'n' ? 'new' : 'old') + ' side)', payload: text.split('\n'), warn: 'If the tool cannot post inline it fails; it never posts a general comment instead.', confirmLabel: 'Post comment',
    exec: () => { addThread(key, inl.pr, inl.file, inl.line, text); S.ui.inline = null; S.ui.drafts['inl:0'] = ''; return { toast: ['Comment posted on ' + pr.repo + ' #' + pr.id, 'ok'] }; } });
}
function askRequestChanges(key, prId) {
  const pr = tk(key).prs.find((p) => p.id === prId);
  openConfirm({ risk: 'medium', title: 'Request changes', ticket: key, needs: 'bkt', auditName: 'bitbucket.pr_request_changes', lead: 'This will request changes on:', payload: [REPOS[pr.repo].host + ' #' + pr.id, '', 'Please address the inline comments before this goes further.'], confirmLabel: 'Request changes',
    exec: () => { requestChanges(key, pr.repo, prId, 'please address the inline comments'); return { toast: ['Changes requested on ' + pr.repo + ' #' + pr.id, 'ok'] }; } });
}
function askStop() {
  const dirty = Object.keys(S.ws.dirty).filter((r) => S.ws.dirty[r]);
  if (!dirty.length) { S.ws.up = false; audit('workspace.stop', null); toast('Workspace stopped', 'info'); return; }
  openConfirm({ risk: 'low', title: 'Stop the workspace', auditName: 'workspace.stop', lead: 'Some projects have work that a stop will not save:', payload: dirty.map((r) => r + ': uncommitted changes'), warn: 'Stopping only brings services down. It does not touch your files.', confirmLabel: 'Stop anyway', exec: () => { S.ws.up = false; audit('workspace.stop', null); return { toast: ['Workspace stopped', 'info'] }; } });
}
function runConfirm() {
  const sh = S.ui.sheet; if (!sh || sh.kind !== 'confirm' || sh.blocked) return;
  if (sh.need && sh.typed.trim().toLowerCase() !== sh.need.toLowerCase()) return;
  const payloadText = (sh.payload || []).join(' | ');
  audit(sh.auditName + '.attempted', sh.ticket, null, 'skipped', payloadText.slice(0, 140));
  if (S.sim.offline && (sh.needs || sh.auditName === 'git.push_uat' && false)) {
    audit(sh.auditName, sh.ticket, null, 'failure', 'network unreachable, nothing sent'); closeSheet(); toast('Network unreachable. Nothing was sent.', 'bad'); return;
  }
  const res = sh.exec(); closeSheet();
  if (res && res.sheet) S.ui.sheet = res.sheet;
  if (res && res.toast) toast(res.toast[0], res.toast[1]);
}
function sheetBody() {
  const sh = S.ui.sheet; if (!sh) return '';
  if (sh.kind === 'confirm') {
    const ok = !sh.blocked && (!sh.need || sh.typed.trim().toLowerCase() === sh.need.toLowerCase());
    return '<div class="sheet ' + sh.risk + '"><div class="sh-h ' + sh.risk + '"><span>' + (sh.risk === 'high' ? '⛔ HIGH RISK · ' : sh.risk === 'medium' ? '⚠ ' : '') + esc(sh.title) + '</span>' + (sh.ticket ? '<span class="key">' + sh.ticket + '</span>' : '') + '</div><div class="sh-b">' +
      (sh.blocked ? '<div class="warn-box">Cannot send: ' + esc(sh.blocked) + '</div>' + btn('Open Settings', 'goSettings', {}, 'sm') :
        '<div>' + esc(sh.lead) + '</div><pre class="code">' + esc(sh.payload.join('\n')) + '</pre>' +
        (sh.details ? '<table class="tbl"><tbody>' + sh.details.map((d) => '<tr><td class="mono">' + esc(d.a) + '</td><td class="mono faint small">' + esc(d.b) + '</td><td>' + esc(d.c) + '</td></tr>').join('') + '</tbody></table>' : '') +
        (sh.checks ? '<div class="row small">' + sh.checks.map((c) => pill(c, 'ok')).join('') + '</div>' : '') +
        (sh.warn ? '<div class="mute"><b>' + esc(sh.warn) + '</b></div>' : '') +
        (sh.need ? '<div class="row"><span>Type <span class="key">' + esc(sh.need) + '</span> to confirm</span><input id="typed" class="in mono" autocomplete="off" spellcheck="false" data-bind="typed:0" value="' + esc(sh.typed) + '"></div>' : '')) +
      '</div><div class="sh-f"><span class="faint small">Draft → confirm → execute. Audited before and after.</span><span class="row">' + btn('Cancel', 'closeSheet', {}) + (sh.blocked ? '' : '<button id="confirmBtn" class="btn ' + (sh.risk === 'high' ? 'd' : 'p') + '" data-act="runConfirm"' + (ok ? '' : ' disabled') + '>' + esc(sh.confirmLabel || 'Confirm') + '</button>') + '</span></div></div>';
  }
  if (sh.kind === 'result') {
    return '<div class="sheet"><div class="sh-h"><span>' + esc(sh.title) + ': result</span><span class="key">' + sh.ticket + '</span></div><div class="sh-b"><table class="tbl"><tbody>' + sh.rows.map((r) => '<tr><td class="mono">' + r.repo + '</td><td>' + pill(r.result, r.result === 'Pushed' || r.result === 'AlreadyPushed' ? 'ok' : 'bad') + '</td><td class="mono faint small">' + esc(r.commit || r.reason || '') + '</td></tr>').join('') + '</tbody></table>' +
      (sh.finalized ? '<div class="card">✓ Every touched repo is on uat. The ticket is now <b>Integrated</b>; your checkouts were restored and the overlay reverted.</div>' : sh.rows.some((r) => r.result === 'Rejected') ? '<div class="warn-box">Rejected: uat moved since you prepared. Nothing was pushed for that repo. Prepare again; do not retry blindly.</div>' : '<div class="warn-box">Not every repo was pushed. Run it again to finish the rest; repos already pushed are skipped.</div>') +
      '</div><div class="sh-f"><span></span>' + btn('Done', 'closeSheet', {}, 'p') + '</div></div>';
  }
  if (sh.kind === 'baseline') {
    return '<div class="sheet"><div class="sh-h"><span>' + esc(sh.key) + ' is a hotfix</span></div><div class="sh-b"><div>Repos this ticket does not touch should run on:</div>' + [['production', 'Production branch (master / main): what the fix will be applied to'], ['base', 'develop: the latest development code'], ['uat', 'uat: what is on alpha now']].map(([v, l]) => '<label class="radio"><input type="radio" name="bl" value="' + v + '" data-act="pickBaseline" ' + (sh.choice === v ? 'checked' : '') + '> ' + esc(l) + '</label>').join('') + '<div class="faint small">Applies to this activation only. Nothing is remembered.</div></div><div class="sh-f"><span></span><span class="row">' + btn('Cancel', 'closeSheet', {}) + btn('Activate', 'activateWith', { key: sh.key }, 'p') + '</span></div></div>';
  }
  if (sh.kind === 'progress') {
    return '<div class="sheet"><div class="sh-h"><span>' + esc(sh.title) + '</span></div><div class="sh-b">' + sh.steps.map((s) => '<div class="ps ' + s.st + '"><i>' + (s.st === 'ok' ? '✓' : s.st === 'run' ? '◐' : s.st === 'warn' ? '!' : '○') + '</i><span><b class="mono">' + esc(s.repo || '') + '</b> ' + esc(s.text) + '</span></div>').join('') + (sh.done ? '<div class="card">' + esc(sh.summary) + '</div>' : '') + '</div><div class="sh-f"><span class="faint small">' + (sh.done ? '' : 'Working…') + '</span>' + btn('Close', 'closeSheet', {}, sh.done ? 'p' : '', { disabled: !sh.done }) + '</div></div>';
  }
  if (sh.kind === 'errors') {
    return '<div class="sheet"><div class="sh-h"><span>' + esc(sh.title) + '</span></div><div class="sh-b">' + sh.errors.map((e) => '<div class="warn-box">' + esc(e) + '</div>').join('') + '</div><div class="sh-f"><span></span>' + btn('Close', 'closeSheet', {}, 'p') + '</div></div>';
  }
  if (sh.kind === 'diagnose') {
    return '<div class="sheet wide"><div class="sh-h"><span>Diagnose providers (fake output)</span></div><div class="sh-b"><div class="warn-box">Real output can contain ticket titles and names. Read it before sharing.</div><pre class="code">$ acli --version\n[exit 0]\nacli version 1.3.39-stable\n\n$ acli jira auth status\n[exit 0]\n✓ Authenticated\n\n$ bkt --version\n[exit 0]\nbkt version 0.32.1\n\n$ bkt auth status --json\n[exit 0]\n' + (S.providers.bkt.ready ? '{ "hosts": [ … ], "contexts": [ … ] }' : '{ "hosts": null, "contexts": null }') + '</pre></div><div class="sh-f"><span></span>' + btn('Close', 'closeSheet', {}, 'p') + '</div></div>';
  }
  if (sh.kind === 'palette') {
    const q = (sh.q || '').toLowerCase();
    const items = [{ l: 'Next', a: { name: 'next' } }, { l: 'On uat', a: { name: 'env' } }, { l: 'Workspace', a: { name: 'workspace' } }, { l: 'Audit log', a: { name: 'audit' } }, { l: 'Settings', a: { name: 'settings' } }, { l: 'Change ideas', a: { name: 'ideas' } }]
      .concat(S.tickets.map((t) => ({ l: t.key + '  ' + t.title, a: { name: 'ticket', key: t.key, tab: 'overview' } }))).filter((i) => !q || i.l.toLowerCase().includes(q)).slice(0, 9);
    return '<div class="sheet pal"><input id="palq" class="in big" placeholder="Jump to a ticket or screen…" data-bind="palq:0" value="' + esc(sh.q || '') + '" autocomplete="off"><div class="pal-l">' + items.map((i) => '<button class="pi" data-act="go"' + attr(i.a) + '>' + esc(i.l) + '</button>').join('') + '</div></div>';
  }
  return '';
}

/* ---------------- progress sheet (visual only; state already changed) ---------------- */
function runProgress(title, steps, summary) {
  const sh = { kind: 'progress', title, steps: steps.map((s) => Object.assign({ st: 'wait' }, s)), done: false, summary };
  S.ui.sheet = sh; let i = 0;
  const tick = () => {
    if (S.ui.sheet !== sh) return;
    if (i > 0 && sh.steps[i - 1]) sh.steps[i - 1].st = sh.steps[i - 1].warn ? 'warn' : 'ok';
    if (i < sh.steps.length) { sh.steps[i].st = 'run'; i++; render(); setTimeout(tick, 420); }
    else { sh.done = true; render(); }
  };
  if (IS_BROWSER) setTimeout(tick, 200); else { sh.steps.forEach((s) => { s.st = 'ok'; }); sh.done = true; }
}
function activationSteps(res) {
  const out = res.rows.map((r) => ({ repo: r.repo, text: (r.role === 'ticket' ? 'switched to ' : 'baseline ') + r.branch + (S.tk_stash && S.tk_stash[r.repo] ? '' : '') }));
  res.records.filter((r) => r.stash).forEach((r) => out.push({ repo: r.repo, text: 'local changes stashed as “' + r.label + '”' }));
  if (res.overlay) { out.push({ repo: res.overlay.repo, text: 'composer path repository → ../' + res.overlay.provider + ', constraint “*”' }); out.push({ repo: res.overlay.repo, text: 'composer update, rebuild build-ui' }); }
  return out;
}

/* ---------------- action dispatcher ---------------- */
function startActivation(key, choice) {
  const r = activate(key, choice);
  if (r.needBaseline) { S.ui.sheet = { kind: 'baseline', key, choice: 'production' }; return; }
  if (!r.ok) { S.ui.sheet = { kind: 'errors', title: 'Cannot activate ' + key, errors: r.errors }; return; }
  runProgress('Activating ' + key, activationSteps(r), 'Active. The touched repos are on their ticket branches; the rest are on their baseline.');
  S.ui.route = { name: 'ticket', key, tab: 'test' };
}
function startDeactivate(key) {
  const r = deactivate(key, 'parked');
  if (!r.ok) { toast(r.errors[0], 'bad'); return; }
  const steps = []; if (r.overlayReverted) steps.push({ repo: r.overlayReverted.repo, text: 'overlay reverted: composer.json and composer.lock restored, composer install' });
  r.restored.forEach((x) => steps.push({ repo: x.repo, text: 'back on ' + x.to + (x.stash ? ', stash popped' : '') }));
  runProgress('Parking ' + key, steps, key + ' is parked. Your repos are exactly as you left them.');
}
function suggestionAct(s) {
  const a = s.act;
  switch (a.do) {
    case 'sync': return ACT.sync();
    case 'claim': claim(a.key); toast('Claimed ' + a.key, 'ok'); return;
    case 'startReview': startReview(a.key); S.ui.route = { name: 'ticket', key: a.key, tab: 'review' }; return;
    case 'markReviewed': markReviewed(a.key); return;
    case 'activate': return startActivation(a.key, null);
    case 'park': return startDeactivate(a.key);
    case 'prepare': prepare(a.key); S.ui.route = { name: 'ticket', key: a.key, tab: 'ship' }; return;
    case 'pushAsk': return askPush(a.key);
    case 'composeDraft': composeDraft(a.key); S.ui.route = { name: 'ticket', key: a.key, tab: 'ship' }; return;
    case 'postAsk': return askPostDraft(a.key, a.draft);
    case 'transitionAsk': return askTransition(a.key);
    case 'approveAsk': return askApprove(a.key, a.repo, a.pr);
    case 'rerunAsk': return askRerun(a.key, a.repo);
    case 'reclaim': reclaim(a.key); toast(a.key + ' claimed again', 'ok'); return;
    case 'settings': S.ui.route = { name: 'settings' }; return;
    case 'open': return ACT.go({ name: 'ticket', key: a.key, tab: a.tab });
  }
}
const ACT = {
  go(d) {
    const name = d.name; const route = { name };
    if (name === 'tickets') route.group = d.group || 'all';
    if (name === 'ticket') { route.key = d.key; route.tab = d.tab || 'overview'; const t = tk(d.key); if (t && route.tab === 'overview') { S.ui.seenSnap = S.ui.seenSnap || {}; if (S.ui.route.key !== d.key || S.ui.route.tab !== 'overview') { S.ui.seenSnap[d.key] = t.seenN; markSeen(d.key); } } }
    S.ui.route = route; S.ui.sheet = null; S.ui.left = false;
  },
  sync() {
    const r = syncStart(); if (!r.ok) return;
    if (IS_BROWSER) setTimeout(() => { const f = syncFinish(); toast(f.ok ? 'Sync finished' : 'Sync failed: cache kept', f.ok ? 'ok' : 'bad'); render(); }, 1300);
    else syncFinish();
  },
  sug(d) { const s = suggest().find((x) => x.id === d.id); if (s) suggestionAct(s); },
  dismissSug(d) { dismiss(d.id); },
  snoozeSug(d) { snooze(d.id, Number(d.m)); },
  undoSug(d) { delete S.responses[d.id]; },
  toggleAllSug() { S.ui.showAllSug = !S.ui.showAllSug; },
  claim(d) { claim(d.key); toast('Claimed ' + d.key, 'ok'); },
  startReview(d) { startReview(d.key); S.ui.route = { name: 'ticket', key: d.key, tab: 'review' }; },
  markReviewed(d) { markReviewed(d.key); toast('Marked reviewed', 'ok'); },
  activate(d) { startActivation(d.key, null); },
  activateWith(d) { const c = S.ui.sheet && S.ui.sheet.choice; S.ui.sheet = null; startActivation(d.key, c); },
  pickBaseline(d, el) { if (S.ui.sheet) S.ui.sheet.choice = el.value; },
  parkAsk(d) { startDeactivate(d.key); },
  choose(d) { setLink(d.key, d.repo, d.branch); },
  toggleChk(d) { toggleChecklist(d.key, Number(d.i)); },
  addChk(d) { const v = S.ui.drafts['chk:' + d.key]; addChecklist(d.key, v); S.ui.drafts['chk:' + d.key] = ''; },
  prepare(d) { const r = prepare(d.key); if (!r.ok) toast(r.error, 'bad'); },
  pushAsk(d) { askPush(d.key); },
  rerunAsk(d) { askRerun(d.key, d.repo); },
  composeDraft(d) { composeDraft(d.key); },
  postAsk(d) { askPostDraft(d.key, d.draft); },
  transitionAsk(d) { askTransition(d.key); },
  approveAsk(d) { askApprove(d.key, d.repo, Number(d.pr)); },
  reqAsk(d) { askRequestChanges(d.key, Number(d.pr)); },
  commentAsk(d) { askComment(d.key); },
  selPr(d) { S.ui.prSel[d.key] = Number(d.pr); S.ui.inline = null; },
  selFile(d) { S.ui.fileSel[d.key + ':' + d.pr] = Number(d.i); S.ui.inline = null; },
  inlineOpen(d) { S.ui.inline = { key: d.key, pr: Number(d.pr), file: d.file, line: d.line }; },
  inlineCancel() { S.ui.inline = null; },
  inlineAsk() { askInline(S.ui.route.key); },
  runConfirm() { runConfirm(); },
  closeSheet() { S.ui.sheet = null; },
  goSettings() { S.ui.sheet = null; S.ui.route = { name: 'settings' }; },
  stopAsk() { askStop(); },
  startWs() { S.ws.up = true; audit('workspace.start', null); toast('Workspace started', 'ok'); },
  provider(d) { SIM.bkt !== undefined && (d.k === 'bkt' ? SIM.bkt(d.v === '1') : SIM.jira(d.v === '1')); toast((d.k === 'bkt' ? 'bkt' : 'acli') + (d.v === '1' ? ' signed in (fake)' : ' signed out (simulated)'), d.v === '1' ? 'ok' : 'warn'); },
  recheck() { toast('Providers re-checked', 'info'); },
  diagnose() { S.ui.sheet = { kind: 'diagnose' }; },
  resetCache() { toast('Cache reset. The next sync rebuilds it (state.db is untouched).', 'info'); },
  resetDemo() { const keep = S.ui; S = newState(); script.length = 0; syncScript().forEach((e) => script.push(e)); S.ui.dev = keep.dev; toast('Demo reset', 'info'); },
  toggleLeft() { S.ui.left = !S.ui.left; },
  toggleRight() { S.ui.right = !S.ui.right; },
  toggleDev() { S.ui.dev = !S.ui.dev; },
  palette() { S.ui.sheet = { kind: 'palette', q: '' }; },
  sim(d) {
    const k = d.k; const key = d.key;
    const map = {
      mention: () => SIM.mention(key), signOff: () => SIM.signOff(key), returned: () => SIM.returned(key), backToReview: () => SIM.backToReview(key), newCommits: () => SIM.newCommits(key),
      offline: () => SIM.toggle('offline', !S.sim.offline), conflict: () => SIM.toggle('conflict', S.sim.conflict ? null : 'web'), leak: () => SIM.toggle('leak', S.sim.leak ? null : 'api-client'),
      uatMoved: () => SIM.toggle('uatMoved', !S.sim.uatMoved), failDeploy: () => SIM.toggle('failDeploy', S.sim.failDeploy ? null : 'web'), bkt: () => SIM.bkt(!S.providers.bkt.ready), jira: () => SIM.jira(!S.providers.jira.ready),
      advance: () => { advance(9000); }, sync: () => { syncNow(); },
    };
    if (map[k]) map[k]();
  },
};
const TEXTINPUT = (name, val) => {
  const [k, a, b] = name.split(':');
  if (k === 'cmt') S.ui.drafts['cmt:' + a] = val;
  else if (k === 'inl') S.ui.drafts['inl:0'] = val;
  else if (k === 'chk') S.ui.drafts['chk:' + a] = val;
  else if (k === 'notes') tk(a).notes = val;
  else if (k === 'draft') { const d = tk(a).drafts.find((x) => x.id === b); if (d) d.body = val; }
  else if (k === 'typed' && S.ui.sheet) S.ui.sheet.typed = val;
  else if (k === 'palq' && S.ui.sheet) S.ui.sheet.q = val;
};

/* ---------------- simulate drawer ---------------- */
function devPanel() {
  if (!S.ui.dev) return '';
  const on = (v) => (v ? 'p' : '');
  const act = activeTicket();
  const anyInt = S.tickets.find((t) => status(t) === 'integrated' && t.merges.length) || S.tickets.find((t) => status(t) === 'integrated');
  const key = anyInt ? anyInt.key : 'PROJ-142';
  return '<aside class="dev"><div class="row sb"><b>Simulate</b><button class="icon" data-act="toggleDev">×</button></div><div class="faint small">Make the outside world do things. Nothing here is real.</div>' +
    '<div class="lab">Environment</div><div class="row wrap">' +
    btn('Offline', 'sim', { k: 'offline' }, on(S.sim.offline) + ' sm') + btn('bkt signed out', 'sim', { k: 'bkt' }, on(!S.providers.bkt.ready) + ' sm') + btn('acli signed out', 'sim', { k: 'jira' }, on(!S.providers.jira.ready) + ' sm') + '</div>' +
    '<div class="lab">Next integration</div><div class="row wrap">' + btn('uat conflict', 'sim', { k: 'conflict' }, on(S.sim.conflict) + ' sm') + btn('overlay leak', 'sim', { k: 'leak' }, on(S.sim.leak) + ' sm') + btn('uat moves before push', 'sim', { k: 'uatMoved' }, on(S.sim.uatMoved) + ' sm') + btn('web deploy fails', 'sim', { k: 'failDeploy' }, on(S.sim.failDeploy) + ' sm') + '</div>' +
    '<div class="lab">Jira events on <span class="key">' + key + '</span></div><div class="row wrap">' + btn('Mention me', 'sim', { k: 'mention', key }, 'sm') + btn('Signed off (Done)', 'sim', { k: 'signOff', key }, 'sm') + btn('Returned', 'sim', { k: 'returned', key }, 'sm') + btn('Back to Review', 'sim', { k: 'backToReview', key }, 'sm') + btn('New commits', 'sim', { k: 'newCommits', key }, 'sm') + '</div>' +
    '<div class="lab">Time</div><div class="row wrap">' + btn('Skip 9s (pipelines)', 'sim', { k: 'advance' }, 'sm') + btn('Run a sync', 'sim', { k: 'sync' }, 'sm') + '</div>' +
    '<div class="lab">Try this</div><ol class="try"><li>Next → <b>Start review</b> on PROJ-142, open Review, <b>Mark reviewed</b>.</li><li>Activate, tick the checklist, <b>Prepare integration</b>, then <b>Review &amp; push</b> and type the key.</li><li>Watch the pipelines; draft, post, transition.</li><li>Then Simulate <b>Signed off</b> and approve.</li><li>Try the conflict, leak and uat-moves switches before pushing.</li></ol></aside>';
}

/* ---------------- render ---------------- */
function toasts() { return '<div class="toasts">' + S.ui.toasts.slice(-4).map((t) => '<div class="toast ' + t.kind + '">' + esc(t.msg) + '</div>').join('') + '</div>'; }
function layout() {
  return '<div class="app ' + (S.ui.right ? '' : 'noright ') + (S.ui.left ? 'showleft' : '') + '"><aside class="left">' + leftNav() + '</aside><div class="center">' + topBar() + nowBar() + '<main class="content">' + mainView() + '</main></div><aside class="right">' + rightPanel() + '</aside></div>' +
    (S.ui.sheet ? '<div class="scrim" data-act="scrim"></div>' + sheetBody() : '') + devPanel() + toasts();
}
function sig() { const r = S.ui.route; return r.name + ':' + (r.key || '') + ':' + (r.tab || '') + ':' + (r.group || ''); }
let lastSig = '';
function render() {
  if (!IS_BROWSER) return;
  const root = document.getElementById('root'); const q = (s) => root.querySelector(s);
  const keep = { c: q('.content') && q('.content').scrollTop, l: q('.left') && q('.left').scrollTop, r: q('.right') && q('.right').scrollTop };
  const ae = document.activeElement; const focusId = ae && ae.id; const selStart = ae && ae.selectionStart;
  root.innerHTML = layout();
  const same = sig() === lastSig; lastSig = sig();
  if (q('.content')) q('.content').scrollTop = same ? keep.c || 0 : 0;
  if (q('.left') && keep.l) q('.left').scrollTop = keep.l; if (q('.right') && keep.r && same) q('.right').scrollTop = keep.r;
  if (focusId) { const el = document.getElementById(focusId); if (el) { el.focus(); try { el.setSelectionRange(selStart, selStart); } catch (e) { /* not a text field */ } } }
  else if (S.ui.sheet && S.ui.sheet.kind === 'palette') { const p = document.getElementById('palq'); if (p) p.focus(); }
}

/* ---------------- boot ---------------- */
function boot() {
  document.addEventListener('click', (e) => {
    const el = e.target.closest('[data-act]'); if (!el || el.disabled) return;
    const act = el.getAttribute('data-act');
    if (act === 'scrim') { S.ui.sheet = null; render(); return; }
    if (el.tagName === 'INPUT' && el.type === 'radio') return;   // handled on change
    if (el.tagName === 'A') e.preventDefault();
    if (el.tagName === 'INPUT' && el.type === 'checkbox') return; // handled on change
    const d = {}; for (const a of el.attributes) if (a.name.startsWith('data-') && a.name !== 'data-act') d[a.name.slice(5)] = a.value;
    if (ACT[act]) { ACT[act](d, el); render(); }
  });
  document.addEventListener('change', (e) => {
    const el = e.target.closest('[data-act]'); if (!el) return;
    if (el.type === 'checkbox' || el.type === 'radio') { const act = el.getAttribute('data-act'); const d = {}; for (const a of el.attributes) if (a.name.startsWith('data-') && a.name !== 'data-act') d[a.name.slice(5)] = a.value; if (ACT[act]) { ACT[act](d, el); render(); } }
  });
  document.addEventListener('input', (e) => {
    const el = e.target; const b = el.getAttribute && el.getAttribute('data-bind'); if (!b) return;
    TEXTINPUT(b, el.value);
    if (b.startsWith('typed:')) { const sh = S.ui.sheet; const btnEl = document.getElementById('confirmBtn'); if (btnEl && sh) btnEl.disabled = !(sh.typed.trim().toLowerCase() === sh.need.toLowerCase()); }
    if (b.startsWith('palq:')) render();
  });
  document.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); S.ui.sheet = { kind: 'palette', q: '' }; render(); return; }
    if (e.key === 'Escape' && S.ui.sheet) { S.ui.sheet = null; render(); return; }
    if (e.key === 'Enter' && S.ui.sheet && S.ui.sheet.kind === 'palette') { const f = document.querySelector('.pal-l .pi'); if (f) f.click(); }
    if (e.key === 'Enter' && e.target && e.target.id === 'chkadd') { const key = S.ui.route.key; ACT.addChk({ key }); render(); }
  });
  render();
  setInterval(() => {
    advance(500);
    const ae = document.activeElement; const typing = ae && (ae.tagName === 'TEXTAREA' || (ae.tagName === 'INPUT' && ae.type !== 'checkbox' && ae.type !== 'radio'));
    if (!typing && !(S.ui.sheet && S.ui.sheet.kind === 'progress')) render();
  }, 500);
}
if (IS_BROWSER) boot();
else Object.assign(globalThis.__DE, { render: () => layout(), ACT, S_get: () => S, mainView, TEXTINPUT, askPush, runConfirm, startActivation, suggestionAct, openConfirm });
