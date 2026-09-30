'use strict';
/* ============================ sheets, actions, boot ============================ */

const IS_BROWSER = typeof document !== 'undefined';

/* ---------------- confirm sheet (the gateway preview) ---------------- */
function openConfirm(o) {
  const blocked = o.needs && !S.providers[o.needs === 'acli' ? 'jira' : 'gh'].ready ? (o.needs === 'acli' ? 'acli' : 'gh') + ' is signed out. Nothing can be sent until you log in.' : null;
  S.ui.sheet = Object.assign({ kind: 'confirm', typed: '', risk: 'medium', blocked }, o);
}
function closeSheet() { S.ui.sheet = null; }

/* a repo is busy: nothing was changed; say who holds it and offer to retry (or remove a stale lock) */
function busySheet(r, retry) { S.ui.sheet = { kind: 'busy', repo: r.busy.repo, holder: r.busy.holder, msg: r.errors[0], retry }; }
function askBreakLock(repo) {
  const h = S.locks[repo]; if (!h || h.kind !== 'stale') { toast('That lock is held by a running process.', 'warn'); return; }
  openConfirm({ risk: 'medium', title: 'Remove a stale git lock', auditName: 'lock.remove_stale', lead: 'This will delete, after checking that no git process is running in ' + repo + ':', payload: [repo + '/.git/index.lock'], warn: 'A crashed git or editor leaves this file behind. Removing it while git is still running can corrupt the index, which is why the check comes first.', confirmLabel: 'Remove lock',
    exec: () => { const r = breakLock(repo); return { toast: [r.ok ? 'Removed the stale lock in ' + repo : r.error, r.ok ? 'ok' : 'bad'] }; } });
}
function askPush(key) {
  const t = tk(key); if (!t.prep || !prepPushable(t)) { toast('Prepare the integration first.', 'warn'); return; }
  const rows = t.prep.rows.filter((r) => r.outcome === 'ready');
  openConfirm({
    risk: 'high', title: 'Push to uat', ticket: key, need: key, auditName: 'git.push_uat',
    lead: 'This will run, exactly:', payload: rows.map((r) => r.repo.padEnd(11) + ' git push origin ' + r.merge + ':refs/heads/uat'),
    details: rows.map((r) => ({ a: r.repo, b: 'uat ' + r.uatBefore + ' → ' + r.merge, c: '+' + r.commits + ' commits · ' + r.files + ' files' })),
    checks: ['overlay guard ✓ (by SHA)', 'no overlay in worktree ✓', 'no force'], warn: 'Pushing deploys to alpha. Nothing has been sent yet.',
    confirmLabel: 'Push to uat', exec: () => { const r = push(key);
      if (r.busy) return { sheet: { kind: 'busy', repo: r.busy.repo, holder: r.busy.holder, msg: r.errors[0], retry: { act: 'pushAsk', args: { key } } } };
      if (!r.ok) return { toast: [r.error, 'bad'] };
      return { sheet: { kind: 'result', title: 'Push to uat', ticket: key, rows: r.results, finalized: r.finalized } }; },
  });
}
function askPostDraft(key, draftId) {
  const t = tk(key); const d = t.drafts.find((x) => x.id === draftId); const move = t.jira === JIRA.review;
  openConfirm({ risk: 'medium', title: move ? 'Post the comment and move the ticket' : 'Post a comment to Jira', ticket: key, needs: 'acli', auditName: 'jira.comment', lead: 'This exact text will be posted as you' + (move ? ', then the ticket is moved:' : ':'), payload: d.body.split('\n').concat(move ? ['', '→ ' + key + '   ' + t.jira + ' → ' + JIRA.alpha] : []), warn: 'Posted comments cannot be recalled from here. Others watch this status.', confirmLabel: move ? 'Post and move' : 'Post comment',
    exec: () => { const r = postAndMove(key, draftId); return { toast: [r.ok ? (move ? key + ' announced and moved to ' + JIRA.alpha : 'Comment posted to ' + key) : r.error, r.ok ? 'ok' : 'bad'] }; } });
}
function askTransition(key) {
  const t = tk(key);
  openConfirm({ risk: 'medium', title: 'Move the ticket in Jira', ticket: key, needs: 'acli', auditName: 'jira.transition', lead: 'This will transition:', payload: [key + '   ' + t.jira + ' → ' + JIRA.alpha], warn: 'Others watch this status.', confirmLabel: 'Transition',
    exec: () => { const r = transition(key); return { toast: [r.ok ? key + ' moved to ' + JIRA.alpha : r.error, r.ok ? 'ok' : 'bad'] }; } });
}
function askApprove(key, repo, pr) {
  openConfirm({ risk: 'medium', title: 'Approve a pull request', ticket: key, needs: 'gh', auditName: 'github.pr_approve', lead: 'This will approve:', payload: [REPOS[repo].host + ' #' + pr], warn: 'Approval is the last gate. It only unlocks after sign-off.', confirmLabel: 'Approve',
    exec: () => { approve(key, repo, pr); return { toast: ['Approved ' + repo + ' #' + pr, 'ok'] }; } });
}
function askApproveAll(key) {
  const t = tk(key); const pend = t.prs.filter((p) => !p.reviewers.find((r) => r.name === ME).approved);
  openConfirm({ risk: 'medium', title: 'Approve ' + pend.length + ' pull requests', ticket: key, needs: 'gh', auditName: 'github.pr_approve', lead: 'This will approve, one after the other:', payload: pend.map((p) => REPOS[p.repo].host + ' #' + p.id + '   ' + p.title), warn: 'Approval is the last gate. It only unlocks after sign-off.', confirmLabel: 'Approve all',
    exec: () => { const r = approveAll(key); return { toast: ['Approved ' + r.n + ' pull requests', 'ok'] }; } });
}
function askReturn(key) {
  const t = tk(key); if (!hasPrGap(t)) { toast('A pull request exists now.', 'ok'); return; }
  openConfirm({ risk: 'medium', title: 'Return to development', ticket: key, needs: 'acli', auditName: 'jira.return', lead: 'This exact comment will be posted as you, then the ticket moves to ' + JIRA.returned + ':', payload: returnBody(t).split('\n').concat(['', '→ ' + key + '   ' + t.jira + ' → ' + JIRA.returned]), warn: 'Jira notifies the developer. This cannot be recalled from here.', confirmLabel: 'Return with comment',
    exec: () => { const r = returnMissingPr(key); return { toast: [r.ok ? key + ' returned with a comment' : r.error, r.ok ? 'ok' : 'bad'] }; } });
}
function askReturnThreads(key) {
  const t = tk(key); if (!blockingThreads(t).length) { toast('Nothing is unresolved now.', 'ok'); return; }
  openConfirm({ risk: 'medium', title: 'Return to development', ticket: key, needs: 'acli', auditName: 'jira.return', lead: 'This exact comment will be posted as you, then the ticket moves to ' + JIRA.returned + ':', payload: returnThreadsBody(t).split('\n').concat(['', '→ ' + key + '   ' + t.jira + ' → ' + JIRA.returned]), warn: 'Jira notifies the developer. This cannot be recalled from here.', confirmLabel: 'Return with comment',
    exec: () => { const r = returnThreads(key); return { toast: [r.ok ? key + ' returned with a comment' : r.error, r.ok ? 'ok' : 'bad'] }; } });
}
function askAcceptThreads(key) {
  const t = tk(key); const bt = blockingThreads(t); if (!bt.length) return;
  openConfirm({ risk: 'low', title: 'Proceed with unresolved comments', ticket: key, auditName: 'threads.accept', lead: 'You are choosing to test and integrate while these are still open:', payload: bt.map(threadLine), warn: 'This stays a warning on the ticket, and the deploy comment lists them. Nothing is sent now.', confirmLabel: 'Proceed anyway',
    exec: () => { const r = acceptThreads(key); return { toast: ['Proceeding with ' + r.n + ' unresolved comment' + (r.n > 1 ? 's' : ''), 'warn'] }; } });
}
function askConflict(key) {
  const t = tk(key); if (!uatConflicts(t).length) { toast('No conflict with uat now.', 'ok'); return; }
  const opt = { label: 'Also return the ticket to development', on: false };
  const build = (on) => ({ payload: conflictBody(t).split('\n').concat(on ? ['', '→ ' + key + '   ' + t.jira + ' → ' + JIRA.returned + (t.act ? '   (your repos are restored first)' : '')] : []), confirmLabel: on ? 'Post and return' : 'Post comment' });
  openConfirm(Object.assign({ risk: 'medium', title: 'Tell the developer about the conflict', ticket: key, needs: 'acli', auditName: 'jira.comment', lead: 'This exact comment will be posted as you:', warn: 'Jira notifies the developer. This cannot be recalled from here.', opt, build,
    exec: () => { const r = sendConflict(key, opt.on); return { toast: [r.ok ? (r.returned ? key + ' returned with a comment' : 'Comment posted to ' + key) : r.error, r.ok ? 'ok' : 'bad'] }; } }, build(false)));
}
function askRerun(key, repo) {
  const d = tk(key).deploy[repo];
  openConfirm({ risk: 'medium', title: 'Re-run a workflow', ticket: key, needs: 'gh', auditName: 'github.actions_rerun', lead: 'This will re-run:', payload: [REPOS[repo].host + ' run #' + d.run + ' (deploy to alpha)'], warn: 'Re-running redeploys to alpha.', confirmLabel: 'Re-run',
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
  openConfirm({ risk: 'medium', title: 'Post an inline PR comment', ticket: key, needs: 'gh', auditName: 'github.pr_comment', lead: REPOS[pr.repo].host + ' #' + pr.id + ' · ' + inl.file + ' line ' + inl.line.slice(1) + ' (' + (inl.line[0] === 'n' ? 'new' : 'old') + ' side)', payload: text.split('\n'), warn: 'If the tool cannot post inline it fails; it never posts a general comment instead.', confirmLabel: 'Post comment',
    exec: () => { addThread(key, inl.pr, inl.file, inl.line, text); S.ui.inline = null; S.ui.drafts['inl:0'] = ''; return { toast: ['Comment posted on ' + pr.repo + ' #' + pr.id, 'ok'] }; } });
}
function askRequestChanges(key, prId) {
  const pr = tk(key).prs.find((p) => p.id === prId);
  openConfirm({ risk: 'medium', title: 'Request changes', ticket: key, needs: 'gh', auditName: 'github.pr_request_changes', lead: 'This will request changes on:', payload: [REPOS[pr.repo].host + ' #' + pr.id, '', 'Please address the inline comments before this goes further.'], confirmLabel: 'Request changes',
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
        (sh.opt ? '<label class="chk big"><input type="checkbox" data-act="toggleOpt"' + (sh.opt.on ? ' checked' : '') + '> ' + esc(sh.opt.label) + '</label>' : '') +
        (sh.warn ? '<div class="mute"><b>' + esc(sh.warn) + '</b></div>' : '') +
        (sh.need ? '<div class="row"><span>Type <span class="key">' + esc(sh.need) + '</span> to confirm</span><input id="typed" class="in mono" autocomplete="off" spellcheck="false" data-bind="typed:0" value="' + esc(sh.typed) + '"></div>' : '')) +
      '</div><div class="sh-f"><span></span><span class="row">' + btn('Cancel', 'closeSheet', {}) + (sh.blocked ? '' : '<button id="confirmBtn" class="btn ' + (sh.risk === 'high' ? 'd' : 'p') + '" data-act="runConfirm"' + (ok ? '' : ' disabled') + '>' + esc(sh.confirmLabel || 'Confirm') + '</button>') + '</span></div></div>';
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
    return '<div class="sheet wide"><div class="sh-h"><span>Diagnose providers (fake output)</span></div><div class="sh-b"><div class="warn-box">Real output can contain ticket titles and names. Read it before sharing.</div><pre class="code">$ acli --version\n[exit 0]\nacli version 1.3.39-stable\n\n$ acli jira auth status\n[exit 0]\n✓ Authenticated\n\n$ gh --version\n[exit 0]\ngh version 2.x\n\n$ gh auth status\n' + (S.providers.gh.ready ? '[exit 0]\ngithub.com\n  ✓ Logged in to github.com' : '[exit 1]\nYou are not logged into any GitHub hosts. To log in, run: gh auth login') + '</pre></div><div class="sh-f"><span></span>' + btn('Close', 'closeSheet', {}, 'p') + '</div></div>';
  }
  if (sh.kind === 'busy') {
    const stale = sh.holder.kind === 'stale';
    return '<div class="sheet"><div class="sh-h"><span>' + esc(sh.repo) + ' is ' + (stale ? 'locked' : 'busy') + '</span></div><div class="sh-b"><div>' + esc(sh.msg) + '</div><div class="mute">' + (stale ? 'A crashed git or editor leaves this behind. The app never removes it without asking.' : 'One thing at a time per repo, so the working tree, stash and branches cannot be changed from two places.') + '</div></div><div class="sh-f">' + btn('Close', 'closeSheet', {}) + (stale ? btn('Remove lock…', 'breakLock', { repo: sh.repo }, 'p') : btn('Retry', 'retryBusy', {}, 'p')) + '</div></div>';
  }
  if (sh.kind === 'claimBlocked') {
    const b = tk(sh.blocker); const what = status(b) === 'active' ? 'active (your repos are on its branches)' : status(b) === 'reviewing' ? 'in review' : 'claimed';
    return '<div class="sheet"><div class="sh-h"><span>One ticket at a time</span></div><div class="sh-b"><div><span class="key">' + sh.blocker + '</span> ' + esc(b.title) + ' is ' + what + '. Finish it, or park it to take <span class="key">' + sh.key + '</span> now.</div>' + (status(b) === 'active' ? '<div class="mute">Parking restores your repos and reverts the overlay. Its notes and checklist stay.</div>' : '<div class="mute">Parking keeps its notes and checklist. You can pick it up again later.</div>') + '</div><div class="sh-f">' + btn('Cancel', 'closeSheet', {}) + btn('Park ' + sh.blocker + ' and continue', 'parkAndClaim', { from: sh.blocker, key: sh.key, then: sh.then }, 'p') + '</div></div>';
  }
  if (sh.kind === 'closeTab') {
    return '<div class="sheet"><div class="sh-h"><span>Close ' + esc(sh.key) + '?</span></div><div class="sh-b">You have an unsent comment on this ticket. Closing the tab discards it.</div><div class="sh-f">' + btn('Keep open', 'closeSheet', {}) + btn('Discard and close', 'closeTab', { key: sh.key, force: 1 }, 'd') + '</div></div>';
  }
  if (sh.kind === 'palette') {
    const q = (sh.q || '').toLowerCase();
    const items = [{ l: 'Next', a: { name: 'next' } }, { l: 'On uat', a: { name: 'env' } }, { l: 'Workspace', a: { name: 'workspace' } }, { l: 'Audit log', a: { name: 'audit' } }, { l: 'Settings', a: { name: 'settings' } }]
      .concat(S.tickets.map((t) => ({ l: t.key + '  ' + t.title, a: { name: 'ticket', key: t.key, tab: 'overview' } }))).filter((i) => !q || i.l.toLowerCase().includes(q)).slice(0, 9);
    return '<div class="sheet pal"><input id="palq" class="in big" placeholder="Go to…" data-bind="palq:0" value="' + esc(sh.q || '') + '" autocomplete="off"><div class="pal-l">' + items.map((i) => '<button class="pi" data-act="go"' + attr(i.a) + '>' + esc(i.l) + '</button>').join('') + '</div></div>';
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
  if (r.busy) { busySheet(r, { act: 'activate', args: { key } }); return; }
  if (r.needBaseline) { S.ui.sheet = { kind: 'baseline', key, choice: 'production' }; return; }
  if (!r.ok) { S.ui.sheet = { kind: 'errors', title: 'Cannot activate ' + key, errors: r.errors }; return; }
  runProgress('Activating ' + key, activationSteps(r), 'Active. The touched repos are on their ticket branches; the rest are on their baseline.');
  S.ui.route = { name: 'ticket', key, tab: 'test' };
}
function startDeactivate(key) {
  const r = deactivate(key, 'parked');
  if (r.busy) { busySheet(r, { act: 'parkAsk', args: { key } }); return; }
  if (!r.ok) { toast(r.errors[0], 'bad'); return; }
  const steps = []; if (r.overlayReverted) steps.push({ repo: r.overlayReverted.repo, text: 'overlay reverted: composer.json and composer.lock restored, composer install' });
  r.restored.forEach((x) => steps.push({ repo: x.repo, text: 'back on ' + x.to + (x.stash ? ', stash popped' : '') }));
  runProgress('Parking ' + key, steps, key + ' is parked. Your repos are exactly as you left them.');
}
function suggestionAct(s) {
  const a = s.act;
  switch (a.do) {
    case 'sync': return ACT.sync();
    case 'claim': return ACT.claim({ key: a.key });
    case 'startReview': return ACT.startReview({ key: a.key });
    case 'markReviewed': return ACT.markReviewed({ key: a.key });
    case 'activate': return startActivation(a.key, null);
    case 'park': return startDeactivate(a.key);
    case 'prepare': { const r = prepare(a.key); if (r.busy) { busySheet(r, { act: 'prepare', args: { key: a.key } }); return; } S.ui.route = { name: 'ticket', key: a.key, tab: 'ship' }; return; }
    case 'breakLock': return askBreakLock(a.repo);
    case 'pushAsk': return askPush(a.key);
    case 'composeDraft': composeDraft(a.key); S.ui.route = { name: 'ticket', key: a.key, tab: 'ship' }; return;
    case 'postAsk': return askPostDraft(a.key, a.draft);
    case 'transitionAsk': return askTransition(a.key);
    case 'returnAsk': return askReturn(a.key);
    case 'returnThreadsAsk': return askReturnThreads(a.key);
    case 'acceptThreadsAsk': return askAcceptThreads(a.key);
    case 'conflictAsk': return askConflict(a.key);
    case 'approveAllAsk': return askApproveAll(a.key);
    case 'approveAsk': return askApprove(a.key, a.repo, a.pr);
    case 'rerunAsk': return askRerun(a.key, a.repo);
    case 'reclaim': { const r = reclaim(a.key); if (r.blocker) { S.ui.sheet = { kind: 'claimBlocked', key: a.key, blocker: r.blocker, then: 'reclaim' }; return; } toast(a.key + ' claimed again', 'ok'); return; }
    case 'settings': S.ui.route = { name: 'settings' }; return;
    case 'open': return ACT.go({ name: 'ticket', key: a.key, tab: a.tab });
  }
}
const ACT = {
  go(d) {
    const name = d.name; const route = { name };
    if (name === 'tickets') route.group = d.group || 'all';
    if (name === 'ticket') { route.key = d.key; route.tab = d.tab || 'overview'; const t = tk(d.key); if (t && route.tab === 'overview') { S.ui.seenSnap = S.ui.seenSnap || {}; if (S.ui.route.key !== d.key || S.ui.route.tab !== 'overview') { S.ui.seenSnap[d.key] = t.seenN; markSeen(d.key); } } }
    S.ui.route = route; S.ui.sheet = null; S.ui.left = false; S.ui.attn = false;
  },
  sync() {
    const r = syncStart(); if (!r.ok) return;
    if (IS_BROWSER) setTimeout(() => { const f = syncFinish(); toast(f.ok ? 'Sync finished' : 'Sync failed: cache kept', f.ok ? 'ok' : 'bad'); render(); }, 1300);
    else syncFinish();
  },
  sug(d) { S.ui.attn = false; const s = suggest().find((x) => x.id === d.id); if (s) suggestionAct(s); },
  dismissSug(d) { dismiss(d.id); toast('Dismissed', 'info', () => { delete S.responses[d.id]; }); },
  snoozeSug(d) { snooze(d.id, Number(d.m)); toast('Snoozed for ' + d.m + ' min', 'info', () => { delete S.responses[d.id]; }); },
  undoSug(d) { delete S.responses[d.id]; },
  toggleAllSug() { S.ui.showAllSug = !S.ui.showAllSug; },
  claim(d) { const r = claim(d.key); if (r.blocker) { S.ui.sheet = { kind: 'claimBlocked', key: d.key, blocker: r.blocker, then: 'claim' }; return; } if (r.ok) toast('Claimed ' + d.key, 'ok', () => unclaim(d.key)); },
  parkAndClaim(d) {
    S.ui.sheet = null; const p = parkInHand(d.from); if (!p.ok) { toast((p.errors || ['Could not park ' + d.from])[0], 'bad'); return; }
    if (d.then === 'reclaim') { reclaim(d.key); toast(d.from + ' parked, ' + d.key + ' claimed again', 'ok'); return; }
    claim(d.key); if (d.then === 'startReview') { startReview(d.key); S.ui.route = { name: 'ticket', key: d.key, tab: 'review' }; }
    toast(d.from + ' parked, ' + d.key + ' claimed', 'ok');
  },
  startReview(d) { const r = startReview(d.key); if (r.blocker) { S.ui.sheet = { kind: 'claimBlocked', key: d.key, blocker: r.blocker, then: 'startReview' }; return; } if (!r.ok) { toast(r.error, 'bad'); return; } S.ui.route = { name: 'ticket', key: d.key, tab: 'review' }; },
  returnAsk(d) { askReturn(d.key); },
  returnThreadsAsk(d) { askReturnThreads(d.key); },
  acceptThreadsAsk(d) { askAcceptThreads(d.key); },
  conflictAsk(d) { askConflict(d.key); },
  thWaitMore(d) { extendThWait(d.key, S.settings.prWaitMin); },
  sugAlt(d) { const s = suggest().find((x) => x.id === d.id); if (s && s.alt) suggestionAct({ act: s.alt }); },
  toggleOpt() { const sh = S.ui.sheet; if (!sh || !sh.opt) return; sh.opt.on = !sh.opt.on; Object.assign(sh, sh.build(sh.opt.on)); },
  prWaitMore(d) { extendPrWait(d.key, S.settings.prWaitMin); },
  prWaitSet(d) { S.settings.prWaitMin = Number(d.m); },
  markReviewed(d) { const t = tk(d.key); const prev = { seq: t.reviewedSeq, status: status(t) }; const r = markReviewed(d.key); if (!r.ok) { toast(r.error, 'bad'); return; } toast('Marked reviewed', 'ok', () => unmarkReviewed(d.key, prev)); },
  activate(d) { startActivation(d.key, null); },
  activateWith(d) { const c = S.ui.sheet && S.ui.sheet.choice; S.ui.sheet = null; startActivation(d.key, c); },
  pickBaseline(d, el) { if (S.ui.sheet) S.ui.sheet.choice = el.value; },
  parkAsk(d) { startDeactivate(d.key); },
  choose(d) { setLink(d.key, d.repo, d.branch); },
  toggleChk(d) { toggleChecklist(d.key, Number(d.i)); },
  addChk(d) { const v = S.ui.drafts['chk:' + d.key]; addChecklist(d.key, v); S.ui.drafts['chk:' + d.key] = ''; },
  prepare(d) { const r = prepare(d.key); if (r.busy) { busySheet(r, { act: 'prepare', args: { key: d.key } }); return; } if (!r.ok) toast(r.error, 'bad'); },
  breakLock(d) { askBreakLock(d.repo); },
  retryBusy(d) { const sh = S.ui.sheet; S.ui.sheet = null; if (sh && sh.retry && ACT[sh.retry.act]) ACT[sh.retry.act](sh.retry.args); },
  pushAsk(d) { askPush(d.key); },
  rerunAsk(d) { askRerun(d.key, d.repo); },
  composeDraft(d) { composeDraft(d.key); },
  postAsk(d) { askPostDraft(d.key, d.draft); },
  transitionAsk(d) { askTransition(d.key); },
  approveAsk(d) { askApprove(d.key, d.repo, Number(d.pr)); },
  reqAsk(d) { askRequestChanges(d.key, Number(d.pr)); },
  commentAsk(d) { askComment(d.key); },
  selPr(d) { S.ui.prSel[d.key] = Number(d.pr); S.ui.inline = null; },
  selFile(d) { S.ui.prSel[d.key] = Number(d.pr); S.ui.fileSel[d.key + ':' + d.pr] = Number(d.i); S.ui.inline = null; },
  toggleViewed(d) { const t = tk(d.key); const pr = t.prs.find((p) => String(p.id) === d.pr.replace(/s$/, '')); const files = d.pr.endsWith('s') ? pr.since.files : pr.files; const k = hunkKey(t, { id: d.pr }, files[Number(d.i)], Number(d.h)); S.ui.viewed[k] = !S.ui.viewed[k]; },
  diffMode(d) { S.ui.diffMode = d.v; },
  sinceMode(d) { S.ui.sinceMode[d.key] = d.v; S.ui.fileSel[d.key + ':' + (S.ui.prSel[d.key] || tk(d.key).prs[0].id)] = 0; },
  togglePre(d) { togglePre(d.key, d.repo, Number(d.i)); },
  approveAllAsk(d) { askApproveAll(d.key); },
  undo(d) { const i = S.ui.toasts.findIndex((x) => String(x.id) === d.id); if (i < 0) return; const u = S.ui.toasts[i].undo; S.ui.toasts.splice(i, 1); if (u) u(); },
  inlineOpen(d) { S.ui.inline = { key: d.key, pr: Number(d.pr), file: d.file, line: d.line }; },
  inlineCancel() { S.ui.inline = null; },
  inlineAsk() { askInline(S.ui.route.key); },
  runConfirm() { runConfirm(); },
  closeSheet() { S.ui.sheet = null; },
  goSettings() { S.ui.sheet = null; S.ui.route = { name: 'settings' }; },
  stopAsk() { askStop(); },
  startWs() { S.ws.up = true; audit('workspace.start', null); toast('Workspace started', 'ok'); },
  provider(d) { SIM.gh !== undefined && (d.k === 'gh' ? SIM.gh(d.v === '1') : SIM.jira(d.v === '1')); toast((d.k === 'gh' ? 'gh' : 'acli') + (d.v === '1' ? ' signed in (fake)' : ' signed out (simulated)'), d.v === '1' ? 'ok' : 'warn'); },
  recheck() { toast('Providers re-checked', 'info'); },
  diagnose() { S.ui.sheet = { kind: 'diagnose' }; },
  resetCache() { toast('Cache reset. The next sync rebuilds it (state.db is untouched).', 'info'); },
  resetDemo() { const keep = S.ui; S = newState(); script.length = 0; syncScript().forEach((e) => script.push(e)); S.ui.dev = keep.dev; toast('Demo reset', 'info'); },
  closeTab(d) {
    const tabs = S.ui.tabs || []; const i = tabs.findIndex((x) => x.key === d.key); if (i < 0) return;
    if (!d.force && (S.ui.drafts['cmt:' + d.key] || '').trim()) { S.ui.sheet = { kind: 'closeTab', key: d.key }; return; }
    tabs.splice(i, 1); S.ui.sheet = null;
    if (S.ui.route.name === 'ticket' && S.ui.route.key === d.key) { const n = tabs[i] || tabs[i - 1]; S.ui.route = n ? { name: 'ticket', key: n.key, tab: n.tab } : (S.ui.screen || { name: 'next' }); }
  },
  pinTab(d) { const t = (S.ui.tabs || []).find((x) => x.key === d.key); if (t) t.pinned = true; },
  cycleTab(d) { const tabs = S.ui.tabs || []; const n = tabs[Number(d.i)]; if (n) S.ui.route = { name: 'ticket', key: n.key, tab: n.tab }; },
  toggleAttn() { S.ui.attn = !S.ui.attn; },
  toggleLeft() { S.ui.left = !S.ui.left; },
  toggleRight() { S.ui.right = !S.ui.right; },
  toggleDev() { S.ui.dev = !S.ui.dev; },
  palette() { S.ui.sheet = { kind: 'palette', q: '' }; },
  sim(d) {
    const k = d.k; const key = d.key;
    const map = {
      mention: () => SIM.mention(key), signOff: () => SIM.signOff(key), returned: () => SIM.returned(key), backToReview: () => SIM.backToReview(key), newCommits: () => SIM.newCommits(key), uatAfterPush: () => SIM.uatAfterPush(key), prArrives: () => SIM.prArrives(d.key2), skipMin: () => SIM.skipMin(30), extLock: () => SIM.extLock(d.key2), staleLock: () => SIM.staleLock(d.key2), resolveThreads: () => SIM.resolveThreads(d.key2),
      offline: () => SIM.toggle('offline', !S.sim.offline), conflict: () => SIM.toggle('conflict', S.sim.conflict ? null : 'web'), leak: () => SIM.toggle('leak', S.sim.leak ? null : 'api-client'),
      uatMoved: () => SIM.toggle('uatMoved', !S.sim.uatMoved), failDeploy: () => SIM.toggle('failDeploy', S.sim.failDeploy ? null : 'web'), gh: () => SIM.gh(!S.providers.gh.ready), jira: () => SIM.jira(!S.providers.jira.ready),
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
  const key = anyInt ? anyInt.key : 'PROJ-142'; const thT = S.tickets.find((t) => blockingThreads(t).length && status(t)); const gapT = S.tickets.find((t) => hasPrGap(t) && status(t)) || S.tickets.find((t) => hasPrGap(t));
  return '<aside class="dev"><div class="row sb"><b>Simulate</b><button class="icon" data-act="toggleDev">×</button></div>' +
    '<div class="lab">Environment</div><div class="row wrap">' +
    btn('Offline', 'sim', { k: 'offline' }, on(S.sim.offline) + ' sm') + btn('gh signed out', 'sim', { k: 'gh' }, on(!S.providers.gh.ready) + ' sm') + btn('acli signed out', 'sim', { k: 'jira' }, on(!S.providers.jira.ready) + ' sm') + '</div>' +
    '<div class="lab">Repo <span class="key">web</span></div><div class="row wrap">' + btn('de CLI holds it', 'sim', { k: 'extLock', key2: 'web' }, on(S.locks.web && S.locks.web.kind === 'external') + ' sm') + btn('stale index.lock', 'sim', { k: 'staleLock', key2: 'web' }, on(S.locks.web && S.locks.web.kind === 'stale') + ' sm') + '</div>' +
    '<div class="lab">Next integration</div><div class="row wrap">' + btn('uat conflict', 'sim', { k: 'conflict' }, on(S.sim.conflict) + ' sm') + btn('overlay leak', 'sim', { k: 'leak' }, on(S.sim.leak) + ' sm') + btn('uat moves before push', 'sim', { k: 'uatMoved' }, on(S.sim.uatMoved) + ' sm') + btn('web deploy fails', 'sim', { k: 'failDeploy' }, on(S.sim.failDeploy) + ' sm') + '</div>' +
    '<div class="lab">Jira events on <span class="key">' + key + '</span></div><div class="row wrap">' + btn('Mention me', 'sim', { k: 'mention', key }, 'sm') + btn('Signed off (Done)', 'sim', { k: 'signOff', key }, 'sm') + btn('Returned', 'sim', { k: 'returned', key }, 'sm') + btn('Back to Review', 'sim', { k: 'backToReview', key }, 'sm') + btn('New commits', 'sim', { k: 'newCommits', key }, 'sm') + btn('uat moved after push', 'sim', { k: 'uatAfterPush', key }, 'sm') + '</div>' +
    (gapT ? '<div class="lab">Missing PR on <span class="key">' + gapT.key + '</span></div><div class="row wrap">' + btn('PR gets opened', 'sim', { k: 'prArrives', key2: gapT.key }, 'sm') + btn('Wait 30 min', 'sim', { k: 'skipMin' }, 'sm') + '</div>' : '') +
    (thT ? '<div class="lab">Unresolved comments on <span class="key">' + thT.key + '</span></div><div class="row wrap">' + btn('Comments get resolved', 'sim', { k: 'resolveThreads', key2: thT.key }, 'sm') + btn('Wait 30 min', 'sim', { k: 'skipMin' }, 'sm') + '</div>' : '') +
    '<div class="lab">Time</div><div class="row wrap">' + btn('Skip 9s (Actions runs)', 'sim', { k: 'advance' }, 'sm') + btn('Run a sync', 'sim', { k: 'sync' }, 'sm') + '</div>' +
    '<div class="lab">Try this</div><ol class="try"><li>Next → <b>Start review</b> on PROJ-142, open Review, <b>Mark reviewed</b>.</li><li>Activate, tick the checklist, <b>Prepare integration</b>, then <b>Review &amp; push</b> and type the key.</li><li>Watch the Actions runs; draft, post, transition.</li><li>Then Simulate <b>Signed off</b> and approve.</li><li>Try the conflict, leak and uat-moves switches before pushing.</li></ol></aside>';
}

/* ---------------- render ---------------- */
function toasts() { return '<div class="toasts">' + S.ui.toasts.slice(-4).map((t) => '<div class="toast ' + t.kind + (t.undo ? ' u' : '') + '">' + esc(t.msg) + (t.undo ? ' <button class="btn sm" data-act="undo" data-id="' + t.id + '">Undo</button>' : '') + '</div>').join('') + '</div>'; }
function layout() {
  syncTabs();
  return '<div class="win">' + titleBar() + '<div class="app ' + (S.ui.right ? '' : 'noright ') + (S.ui.left ? 'showleft' : '') + '"><aside class="left">' + leftNav() + '</aside><div class="center">' + tabBar() + '<main class="content">' + mainView() + '</main></div><aside class="right">' + rightPanel() + '</aside></div>' + statusBar() + '</div>' +
    attnPanel() + (S.ui.sheet ? '<div class="scrim" data-act="scrim"></div>' + sheetBody() : '') + devPanel() + toasts();
}
/* keyboard: j/k move through the cards or rows on the screen, Enter runs the primary action */
function navItems() { return IS_BROWSER ? Array.from(document.querySelectorAll('.content [data-nav]')) : []; }
function applyCursor() {
  const its = navItems(); its.forEach((e) => e.classList.remove('cur'));
  if (S.ui.cursor != null && its[S.ui.cursor]) { its[S.ui.cursor].classList.add('cur'); its[S.ui.cursor].scrollIntoView({ block: 'nearest' }); }
}
function moveCursor(dir) { const n = navItems().length; if (!n) return; S.ui.cursor = S.ui.cursor == null ? (dir > 0 ? 0 : n - 1) : Math.max(0, Math.min(n - 1, S.ui.cursor + dir)); }
function activateCursor() { const el = navItems()[S.ui.cursor]; if (!el) return false; (el.tagName === 'TR' ? el : el.querySelector('.btn.p') || el.querySelector('.btn')).click(); return true; }
function sig() { const r = S.ui.route; return r.name + ':' + (r.key || '') + ':' + (r.tab || '') + ':' + (r.group || ''); }
let lastSig = ''; const scrollOf = {};
function render() {
  if (!IS_BROWSER) return;
  const root = document.getElementById('root'); const q = (s) => root.querySelector(s);
  const keep = { c: q('.content') && q('.content').scrollTop, l: q('.left') && q('.left').scrollTop, r: q('.right') && q('.right').scrollTop };
  const ae = document.activeElement; const focusId = ae && ae.id; const selStart = ae && ae.selectionStart;
  root.innerHTML = layout();
  scrollOf[lastSig] = keep.c || 0;
  const same = sig() === lastSig; lastSig = sig(); if (!same) S.ui.cursor = null;
  if (q('.content')) q('.content').scrollTop = same ? keep.c || 0 : scrollOf[lastSig] || 0;
  if (q('.left') && keep.l) q('.left').scrollTop = keep.l; if (q('.right') && keep.r && same) q('.right').scrollTop = keep.r;
  applyCursor();
  if (focusId) { const el = document.getElementById(focusId); if (el) { el.focus(); try { el.setSelectionRange(selStart, selStart); } catch (e) { /* not a text field */ } } }
  else if (S.ui.sheet && S.ui.sheet.kind === 'palette') { const p = document.getElementById('palq'); if (p) p.focus(); }
}

/* acting on a ticket turns its preview tab into a permanent one */
const NAV = new Set(['go', 'selPr', 'selFile', 'inlineCancel', 'closeSheet', 'closeTab', 'pinTab', 'cycleTab', 'toggleAttn', 'retryBusy', 'prWaitSet', 'toggleOpt', 'toggleViewed', 'diffMode', 'sinceMode', 'undo', 'toggleLeft', 'toggleRight', 'toggleDev', 'palette', 'sim', 'sync', 'toggleAllSug', 'undoSug', 'goSettings', 'resetDemo', 'provider', 'recheck', 'diagnose', 'resetCache']);
Object.keys(ACT).forEach((k) => { if (NAV.has(k)) return; const f = ACT[k]; ACT[k] = function (d, el) { const r = f.call(this, d, el); const key = d && d.key; const t = key && (S.ui.tabs || []).find((x) => x.key === key); if (t) t.pinned = true; return r; }; });

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
  document.addEventListener('dblclick', (e) => { const el = e.target.closest('[data-dbl]'); if (el) { ACT.pinTab({ key: el.getAttribute('data-key') }); render(); } });
  document.addEventListener('auxclick', (e) => { const el = e.target.closest('[data-mid]'); if (el && e.button === 1) { e.preventDefault(); ACT.closeTab({ key: el.getAttribute('data-key') }); render(); } });
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
    const typing = e.target && (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA' || e.target.tagName === 'SELECT');
    if (!typing && !e.metaKey && !e.ctrlKey && !e.altKey && !S.ui.sheet && !S.ui.attn) {
      if (e.key === 'j' || e.key === 'ArrowDown') { e.preventDefault(); moveCursor(1); render(); return; }
      if (e.key === 'k' || e.key === 'ArrowUp') { e.preventDefault(); moveCursor(-1); render(); return; }
      if (e.key === 'Enter' && S.ui.cursor != null && activateCursor()) { e.preventDefault(); return; }
    }
    if (e.altKey && e.key.toLowerCase() === 'w' && S.ui.route.name === 'ticket') { e.preventDefault(); ACT.closeTab({ key: S.ui.route.key }); render(); return; }
    if (e.altKey && /^[1-9]$/.test(e.key)) { e.preventDefault(); ACT.cycleTab({ i: Number(e.key) - 1 }); render(); return; }
    if (e.key === 'Escape' && S.ui.attn && !S.ui.sheet) { S.ui.attn = false; render(); return; }
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
