'use strict';
/* ============================ screens ============================ */

/* ---------------- Next ---------------- */
function sugCard(s) {
  const lvl = s.level === 'external' ? pill('sends to a remote', 'bad') : s.level === 'auto' ? pill('automatic', '') : pill('local', '');
  const labelFor = { sync: 'Sync now', claim: 'Claim', startReview: 'Start review', activate: 'Activate…', prepare: 'Prepare integration', pushAsk: 'Review & push to uat…', composeDraft: 'Draft comment', postAsk: 'Review & post…', transitionAsk: 'Transition…', approveAsk: 'Approve…', rerunAsk: 'Re-run…', open: 'Open', park: 'Park…', reclaim: 'Claim again', settings: 'Open Settings', markReviewed: 'Mark reviewed' };
  const primary = btn(labelFor[s.act.do] || 'Do', 'sug', { id: s.id }, s.level === 'external' ? 'p' : s.info ? '' : 'p');
  const extra = s.act.do === 'open' || s.act.do === 'rerunAsk' ? '' : '';
  return '<article class="card sug ' + (s.state === 'resurfaced' ? 'res' : '') + '"><div class="rk">' + s.rank + '</div><div class="sb-main">' +
    '<div class="row">' + (s.hotfix ? pill('HOTFIX', 'hot') : '') + (s.info ? pill('info', '') : '') + (s.ticket ? '<a href="#" class="key" data-act="go" data-name="ticket" data-key="' + s.ticket + '" data-tab="overview">' + s.ticket + '</a>' : '') + '<b class="tt">' + esc(s.title) + '</b>' + (s.state === 'resurfaced' ? '<span class="dotnew" title="Came back because the facts changed"></span>' : '') + '</div>' +
    '<div class="why">' + esc(s.reason) + '</div>' +
    '<div class="row sb"><span class="row">' + primary + extra + lvl + '</span><span class="row small"><span class="faint">Snooze</span>' + btn('2m', 'snoozeSug', { id: s.id, m: 2 }, 'g sm') + btn('10m', 'snoozeSug', { id: s.id, m: 10 }, 'g sm') + btn('Dismiss', 'dismissSug', { id: s.id }, 'g sm') + '</span></div></div></article>';
}
function viewNext() {
  const all = suggest(); const shown = S.ui.showAllSug ? all : all.filter((s) => s.state === 'open' || s.state === 'resurfaced');
  const hidden = all.length - all.filter((s) => s.state === 'open' || s.state === 'resurfaced').length;
  return '<div class="row sb"><h2>Next</h2><label class="chk"><input type="checkbox" data-act="toggleAllSug" ' + (S.ui.showAllSug ? 'checked' : '') + '> show dismissed and snoozed (' + hidden + ')</label></div>' +
    '<p class="lede">Ranked by what unblocks you first. Each card says why it exists. Buttons that send something to a remote end in “…” and open a preview you must confirm.</p>' +
    (shown.length ? '<div class="stack">' + shown.map((s) => s.state === 'dismissed' || s.state === 'snoozed' ? '<div class="dim-card">' + sugCard(s).replace('<article class="card sug', '<article class="card sug muted') + '<div class="small faint pad">' + s.state + ' · ' + btn('Undo', 'undoSug', { id: s.id }, 'g sm') + '</div></div>' : sugCard(s)).join('') + '</div>'
      : '<div class="empty"><b>Nothing to do.</b><div>Everything is synced and no ticket needs you. Sync to look for new work.</div>' + btn('↻ Sync', 'sync', {}, 'p') + '</div>');
}

/* ---------------- ticket list ---------------- */
function ticketRow(t) {
  const st = status(t); const un = unseenCount(t); const rv = t.reviewed && t.prs.some((p) => p.updatedSeq > t.reviewedSeq);
  const d = t.deploy; const dep = t.merges.length ? (allDeployed(t) ? pill('deployed', 'ok') : anyFailed(t).length ? pill('failed', 'bad') : pill('deploying', 'warn')) : '';
  return '<tr class="hover" data-act="go" data-name="ticket" data-key="' + t.key + '" data-tab="overview"><td class="key">' + t.key + '</td><td>' + pill(t.priority, prioCls(t.priority)) + '</td><td><b>' + esc(t.title) + '</b>' + (isHotfix(t) ? ' ' + pill('HOTFIX', 'hot') : '') + '<div class="mute small">' + esc(t.type) + ' · ' + esc(t.assignee) + '</div></td><td>' + pill(t.jira, jiraCls(t.jira)) + '</td><td>' + (st ? pill(LOCAL_LABEL[st], localCls(st)) : '<span class="faint">unclaimed</span>') + '</td><td>' + t.prs.map((p) => '<span class="mono">' + p.repo + '</span>').join(' ') + '</td><td>' + dep + (un ? ' ' + pill(un + ' new', 'acc') : '') + (rv ? ' ' + pill('new commits', 'warn') : '') + '</td></tr>';
}
function viewTickets() {
  const g = S.ui.route.group || 'all';
  const rows = S.tickets.filter(groupOf[g]).sort((a, b) => (PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority]) || a.key.localeCompare(b.key, undefined, { numeric: true }));
  return '<h2>' + GROUP_LABEL[g] + '</h2><p class="lede">Ordered by Jira priority. “new” counts comments and commits since you last opened the ticket.</p>' +
    '<div class="tabs sm">' + ['pool', 'mine', 'active', 'parked', 'awaiting', 'returned', 'done', 'all'].map((k) => '<a href="#" data-act="go" data-name="tickets" data-group="' + k + '" class="' + (k === g ? 'on' : '') + '">' + GROUP_LABEL[k] + ' <b>' + count(k) + '</b></a>').join('') + '</div>' +
    (rows.length ? '<div class="tbwrap"><table class="tbl"><thead><tr><th>Key</th><th>Priority</th><th>Title</th><th>Jira</th><th>Local</th><th>Repos</th><th></th></tr></thead><tbody>' + rows.map(ticketRow).join('') + '</tbody></table></div>' : '<div class="empty"><b>No tickets here.</b></div>');
}

/* ---------------- ticket detail ---------------- */
function ticketHead(t, tab) {
  const st = status(t); const hot = isHotfix(t);
  const acts = [];
  if (!st) acts.push(btn('Claim', 'claim', { key: t.key }, 'p'));
  if (st === 'claimed') acts.push(btn('Start review', 'startReview', { key: t.key }, 'p'));
  if (['claimed', 'reviewing', 'parked'].includes(st) && t.reviewed) acts.push(btn('Activate…', 'activate', { key: t.key }, 'p'));
  if (st === 'reviewing' && !t.reviewed) acts.push(btn('Mark reviewed', 'markReviewed', { key: t.key }, 'p'));
  if (st === 'active') acts.push(btn('Park…', 'parkAsk', { key: t.key }));
  const tabs = ['overview', 'review', 'test', 'ship', 'timeline'];
  const un = unseenCount(t) - (S.ui.seenSnap && S.ui.seenSnap[t.key] != null ? 0 : 0);
  return '<div class="thead"><div class="row sb"><div><div class="row"><span class="key big">' + t.key + '</span>' + pill(t.type) + pill(t.priority, prioCls(t.priority)) + (hot ? pill('HOTFIX', 'hot') : '') + pill(t.jira, jiraCls(t.jira)) + (st ? pill(LOCAL_LABEL[st], localCls(st)) : pill('not claimed')) + '</div><h2 class="tt2">' + esc(t.title) + '</h2></div><div class="row">' + acts.join('') + '</div></div>' + stepper(t) +
    '<div class="tabs">' + tabs.map((k) => '<a href="#" data-act="go" data-name="ticket" data-key="' + t.key + '" data-tab="' + k + '" class="' + (k === tab ? 'on' : '') + '">' + ({ overview: 'Overview', review: 'Review', test: 'Test', ship: 'Ship', timeline: 'Timeline' })[k] + (k === 'review' && t.reviewed && t.prs.some((p) => p.updatedSeq > t.reviewedSeq) ? ' <i class="dotnew"></i>' : '') + '</a>').join('') + '</div></div>';
}

function reposTable(t) {
  const plan = repoPlan(t); const base = Object.keys(REPOS).filter((r) => !plan.find((p) => p.repo === r));
  const row = (p) => {
    const pr = t.prs.find((x) => x.repo === p.repo); const d = t.deploy[p.repo];
    const br = p.ambiguous ? '<div class="warn-box">⚠ ' + p.cands.length + ' branches match. Choose one:<div class="row">' + p.cands.map((c) => btn(esc(c), 'choose', { key: t.key, repo: p.repo, branch: c }, 'sm')).join('') + '</div></div>' : '<span class="mono">' + esc(p.chosen || '–') + '</span>' + (p.manual ? ' ' + pill('chosen', '') : '');
    return '<tr><td class="mono">' + p.repo + '</td><td>' + br + '</td><td>' + (pr ? '#' + pr.id + ' <span class="faint small">→ ' + esc(pr.dst) + '</span>' : '<span class="faint">–</span>') + '</td><td>' + (d ? pill(d.state === 'deployed' ? 'deployed alpha' : d.state, d.state === 'deployed' ? 'ok' : d.state === 'failed' ? 'bad' : 'warn') + ' <span class="faint small">#' + d.run + '</span>' : '<span class="faint">not pushed</span>') + '</td></tr>';
  };
  return '<div class="tbwrap"><table class="tbl"><thead><tr><th>Repo</th><th>Branch</th><th>PR</th><th>Deploy</th></tr></thead><tbody>' + plan.map(row).join('') +
    base.map((r) => '<tr class="faint"><td class="mono">' + r + '</td><td class="mono">' + REPOS[r].base + ' (baseline)</td><td>–</td><td>not touched</td></tr>').join('') + '</tbody></table></div>';
}
function tabOverview(t) {
  const snap = S.ui.seenSnap && S.ui.seenSnap[t.key] != null ? S.ui.seenSnap[t.key] : t.seenN;
  const comments = t.comments.length ? t.comments.map((c) => '<div class="cmt ' + (c.n > snap && c.who !== ME ? 'new' : '') + '"><div class="row sb"><span class="row"><span class="av">' + esc(c.who.split(' ').map((x) => x[0]).join('')) + '</span><b>' + esc(c.who) + '</b><span class="faint small">' + esc(c.at) + '</span></span>' + (c.n > snap && c.who !== ME ? pill('new', 'acc') : '') + '</div><div class="cbody">' + blocksHtml(c.body) + '</div></div>').join('') : '<div class="faint">No comments yet.</div>';
  return '<div class="grid2"><div class="col">' +
    '<section class="sec"><h3>Description</h3><div class="prose">' + blocksHtml(t.desc) + '</div></section>' +
    '<section class="sec"><h3>Acceptance criteria</h3><ul class="ac">' + t.ac.map((a) => '<li>☐ ' + esc(a) + '</li>').join('') + '</ul></section>' +
    (t.subtasks.length ? '<section class="sec"><h3>Subtasks</h3><ul class="ac">' + t.subtasks.map((s) => '<li>' + (s.done ? '☑' : '☐') + ' ' + esc(s.title) + '</li>').join('') + '</ul></section>' : '') +
    '<section class="sec"><h3>Repos and pull requests</h3>' + reposTable(t) + '</section>' +
    '<section class="sec"><h3>Comments <span class="faint small">(' + t.comments.length + ', from Jira)</span></h3>' + comments +
    '<div class="composer"><textarea id="cmt" rows="3" placeholder="Add a comment to Jira…" data-bind="cmt:' + t.key + '">' + esc(S.ui.drafts['cmt:' + t.key] || '') + '</textarea><div class="row sb"><span class="faint small">Posting goes through the confirm sheet.</span>' + btn('Post to Jira…', 'commentAsk', { key: t.key }, 'p') + '</div></div></section></div>' +
    '<div class="col side">' +
    (t.attachments.length ? '<section class="sec"><h3>Attachments</h3>' + t.attachments.map((a) => '<div class="att"><span class="fileic">▤</span><div><div>' + esc(a.name) + '</div><div class="faint small">' + esc(a.size) + '</div></div></div>').join('') + '</section>' : '') +
    '<section class="sec"><h3>Your notes</h3><textarea rows="5" placeholder="Private notes, saved locally…" data-bind="notes:' + t.key + '">' + esc(t.notes) + '</textarea><div class="faint small">Local only. Never sent to Jira.</div></section><div class="narrow-only">' + rightTicket(t) + '</div></div></div>';
}

/* ---- review ---- */
function diffView(t, pr, fi) {
  const f = pr.files[fi]; const threads = pr.threads.filter((th) => th.file === f.path);
  const comp = S.ui.inline && S.ui.inline.pr === pr.id && S.ui.inline.file === f.path ? S.ui.inline : null;
  const rows = f.lines.map((l) => {
    const anchor = l.n != null ? 'n' + l.n : 'o' + l.o; const ths = threads.filter((th) => th.line === anchor);
    let out = '<div class="dl ' + l.k + '"><span class="ln">' + (l.o == null ? '' : l.o) + '</span><span class="ln">' + (l.n == null ? '' : l.n) + '</span><span class="sg">' + (l.k === 'add' ? '+' : l.k === 'del' ? '−' : '') + '</span><code>' + esc(l.x) + '</code><button class="plus" data-act="inlineOpen" data-key="' + t.key + '" data-pr="' + pr.id + '" data-file="' + esc(f.path) + '" data-line="' + anchor + '" title="Comment on this line">+</button></div>';
    ths.forEach((th) => { out += '<div class="thread"><div class="row"><span class="av">' + esc(th.author.split(' ').map((x) => x[0]).join('')) + '</span><b>' + esc(th.author) + '</b>' + (th.author === ME ? pill('you', 'acc') : '') + '</div><div>' + esc(th.text) + '</div></div>'; });
    if (comp && comp.line === anchor) out += '<div class="thread new"><textarea id="inl" rows="3" placeholder="Comment on ' + esc(f.path) + ' line ' + anchor.slice(1) + '…" data-bind="inl:0">' + esc(S.ui.drafts['inl:0'] || '') + '</textarea><div class="row">' + btn('Comment…', 'inlineAsk', {}, 'p sm') + btn('Cancel', 'inlineCancel', {}, 'g sm') + '</div></div>';
    return out;
  }).join('');
  return '<div class="diff">' + rows + '</div>';
}
function tabReview(t) {
  if (!t.prs.length) return '<div class="empty">No pull requests for this ticket.</div>';
  const sel = S.ui.prSel[t.key] != null && t.prs.find((p) => p.id === S.ui.prSel[t.key]) ? t.prs.find((p) => p.id === S.ui.prSel[t.key]) : t.prs[0];
  const fi = Math.min(S.ui.fileSel[t.key + ':' + sel.id] || 0, sel.files.length - 1);
  const me = sel.reviewers.find((r) => r.name === ME); const canApprove = isSignedOff(t) && S.providers.bkt.ready && !me.approved;
  const stale = t.reviewed && t.prs.some((p) => p.updatedSeq > t.reviewedSeq);
  return (stale ? '<div class="warn-box">⚠ New commits arrived since you reviewed this ticket. ' + btn('Start review again', 'startReview', { key: t.key }, 'sm') + '</div>' : '') +
    '<div class="row sb"><div class="row">' + t.prs.map((p) => '<button class="chip ' + (p.id === sel.id ? 'on' : '') + '" data-act="selPr" data-key="' + t.key + '" data-pr="' + p.id + '">' + p.repo + ' #' + p.id + '</button>').join('') + '</div><div class="row small"><span class="faint">unified</span></div></div>' +
    '<div class="prhead"><b>' + esc(sel.title) + '</b><div class="mute">' + esc(sel.src) + ' → ' + pill(sel.dst, sel.dst === REPOS[sel.repo].prod && REPOS[sel.repo].prod !== REPOS[sel.repo].base ? 'hot' : '') + ' <span class="faint small">diffed against the PR destination, from local git, nothing checked out</span></div><div class="row small">Reviewers: ' + sel.reviewers.map((r) => pill(r.name + (r.approved ? ' ✓' : ''), r.approved ? 'ok' : '')).join(' ') + '</div></div>' +
    '<div class="rvgrid"><div class="ftree"><div class="lab">Files (' + sel.files.length + ')</div>' + sel.files.map((f, i) => '<button class="fl ' + (i === fi ? 'on' : '') + '" data-act="selFile" data-key="' + t.key + '" data-pr="' + sel.id + '" data-i="' + i + '"><span class="mono">' + esc(f.path) + '</span><em><i class="ad">+' + f.adds + '</i> <i class="dl2">−' + f.dels + '</i></em></button>').join('') + '<div class="lab">Threads (' + sel.threads.length + ')</div><div class="faint small">' + sel.threads.length + ' inline comment' + (sel.threads.length === 1 ? '' : 's') + '</div></div><div>' +
    '<div class="mono fpath">' + esc(sel.files[fi].path) + '</div>' + diffView(t, sel, fi) + '</div></div>' +
    '<div class="row rvbar">' + btn(t.reviewed ? '✓ Reviewed' : 'Mark reviewed', 'markReviewed', { key: t.key }, t.reviewed ? '' : 'p', { disabled: t.reviewed && !stale }) + btn('Request changes…', 'reqAsk', { key: t.key, pr: sel.id }) + btn('Approve…', 'approveAsk', { key: t.key, repo: sel.repo, pr: sel.id }, 'p', { disabled: !canApprove }) +
    '<span class="faint small">' + (me.approved ? 'You approved this PR.' : !isSignedOff(t) ? 'Approval unlocks when the ticket reaches a signed-off status (now: ' + esc(t.jira) + ').' : !S.providers.bkt.ready ? 'bkt is signed out.' : '') + '</span></div>';
}

/* ---- test ---- */
function tabTest(t) {
  const st = status(t);
  if (st !== 'active') {
    const why = !st ? 'Claim and review this ticket first.' : !t.reviewed && st !== 'integrated' ? 'Finish the review, then activate to test locally.' : st === 'integrated' ? 'This ticket is already on uat. Activate it again only for a fix after alpha.' : 'Activating switches the touched repos to the ticket branches, stashes local changes, and applies the test overlay where needed.';
    const plan = activationPlan(t, null);
    return '<div class="empty left"><b>Not active.</b><div>' + esc(why) + '</div>' + (plan.errors.length ? '<div class="warn-box">' + plan.errors.map(esc).join('<br>') + '</div>' : '') +
      '<div class="lab">If you activate now</div><div class="tbwrap"><table class="tbl"><tbody>' + plan.rows.map((r) => '<tr><td class="mono">' + r.repo + '</td><td>' + (r.role === 'ticket' ? pill('ticket branch', 'acc') : pill('baseline', '')) + '</td><td class="mono">' + esc(r.branch) + '</td><td class="faint small">was ' + esc(S.ws.branches[r.repo]) + (S.ws.dirty[r.repo] ? ', local changes stashed' : '') + '</td></tr>').join('') + '</tbody></table></div>' +
      (plan.overlay ? '<div class="card overlay">⚡ Test overlay in <b>' + plan.overlay.repo + '</b>: composer path repository → ../' + plan.overlay.provider + ', constraint “*”. Reverted when you park or finish.</div>' : '') +
      (['claimed', 'reviewing', 'parked', 'integrated'].includes(st) ? '<div class="row">' + btn('Activate…', 'activate', { key: t.key }, 'p') + '</div>' : '') + '</div>';
  }
  const done = t.checklist.filter((c) => c.done).length;
  return '<div class="grid2"><div class="col"><section class="sec"><div class="row sb"><h3>Checklist</h3>' + pill(done + ' of ' + t.checklist.length, done === t.checklist.length && done ? 'ok' : '') + '</div>' +
    t.checklist.map((c, i) => '<label class="chk big"><input type="checkbox" data-act="toggleChk" data-key="' + t.key + '" data-i="' + i + '" ' + (c.done ? 'checked' : '') + '> <span class="' + (c.done ? 'strike' : '') + '">' + esc(c.text) + '</span></label>').join('') +
    '<div class="row"><input id="chkadd" class="in" placeholder="Add an item…" data-bind="chk:' + t.key + '"> ' + btn('Add', 'addChk', { key: t.key }, 'sm') + '</div></section>' +
    '<section class="sec"><h3>Notes</h3><textarea rows="5" data-bind="notes:' + t.key + '" placeholder="What you verified, what broke…">' + esc(t.notes) + '</textarea></section></div>' +
    '<div class="col side"><section class="sec"><h3>Environment</h3><div class="tbwrap"><table class="tbl"><tbody>' + t.act.records.map((r) => '<tr><td class="mono">' + r.repo + '</td><td>' + (r.role === 'ticket' ? pill('ticket', 'acc') : pill('baseline')) + '</td><td class="mono">' + esc(r.branch) + '</td></tr>' + (r.stash ? '<tr class="faint"><td></td><td colspan="2" class="small">stash “' + esc(r.label) + '”, was on ' + esc(r.prev) + '</td></tr>' : '')).join('') + '</tbody></table></div>' +
    (t.act.overlay ? '<div class="card overlay"><b>⚡ Test overlay applied in ' + t.act.overlay.repo + '</b><div class="small">composer path repository → ../' + t.act.overlay.provider + ', constraint “*”. Rebuild ran. <b>Never pushed:</b> reverted when you park or finish, and blocked by a guard if it reaches a commit.</div></div>' : '<div class="faint small">No overlay needed: the API client is not on a ticket branch.</div>') + '</section>' +
    '<section class="sec"><h3>Next</h3><div class="stack">' + btn('Prepare integration →', 'go', { name: 'ticket', key: t.key, tab: 'ship' }, 'p') + btn('Park…', 'parkAsk', { key: t.key }) + '</div></section></div></div>';
}

/* ---- ship ---- */
function pipeChip(t, repo) {
  const d = t.deploy[repo]; if (!d) return '<span class="faint">not pushed</span>';
  const cls = d.state === 'deployed' ? 'ok' : d.state === 'failed' ? 'bad' : 'warn';
  return pill((d.state === 'deployed' ? '● deployed alpha' : d.state === 'failed' ? '✕ failed' : '◐ ' + d.state), cls) + ' <span class="faint small">#' + d.run + (d.state === 'running' || d.state === 'pending' ? ' · ' + d.step : '') + '</span>';
}
function tabShip(t) {
  const st = status(t); const pushedAll = touchedRepos(t).length && touchedRepos(t).every((p) => t.merges.some((m) => m.repo === p.repo));
  const s1 = st === 'active' ? 'now' : pushedAll ? 'done' : '';
  let step1;
  if (st === 'active') {
    const prep = t.prep;
    step1 = (prep ? '<div class="tbwrap"><table class="tbl"><tbody>' + prep.rows.map((r) => {
      if (r.outcome === 'ready') return '<tr><td class="mono">' + r.repo + '</td><td>' + pill('merge ready', 'ok') + '</td><td>+' + r.commits + ' commits · ' + r.files + ' files</td><td class="mono faint small">uat ' + r.uatBefore + ' → ' + r.merge + '</td></tr>' + (r.overlaps.length ? '<tr><td></td><td colspan="3"><div class="warn-box">⚠ Overlaps with ' + r.overlaps.map((o) => '<b>' + o.key + '</b> (' + esc(o.file) + ')').join(', ') + ' already on uat. Not a conflict, but check the combined behaviour.</div></td></tr>' : '');
      if (r.outcome === 'alreadyPushed') return '<tr><td class="mono">' + r.repo + '</td><td>' + pill('already pushed', 'ok') + '</td><td colspan="2" class="mono faint small">' + r.commit + '</td></tr>';
      if (r.outcome === 'conflict') return '<tr><td class="mono">' + r.repo + '</td><td>' + pill('conflict', 'bad') + '</td><td colspan="2">Conflicts with <b>' + r.with + '</b> in <span class="mono">' + esc(r.files.join(', ')) + '</span>. The merge was aborted; nothing was pushed.</td></tr>';
      return '<tr><td class="mono">' + r.repo + '</td><td>' + pill('blocked', 'bad') + '</td><td colspan="2">' + esc(r.reason) + '</td></tr>';
    }).join('') + '</tbody></table></div><div class="row small">' + pill('overlay guard ✓ by SHA', 'ok') + pill('no overlay in worktree ✓', 'ok') + '</div>' : '<div class="mute">Preparing merges each touched repo into a temporary worktree of uat and checks it. It pushes nothing.</div>') +
      '<div class="row">' + btn(prep ? 'Refresh preparation' : 'Prepare integration', 'prepare', { key: t.key }, prep ? '' : 'p') + btn('Review & push to uat…', 'pushAsk', { key: t.key }, 'p', { disabled: !prepPushable(t) }) + '</div>';
  } else if (pushedAll) {
    step1 = '<div class="tbwrap"><table class="tbl"><tbody>' + t.merges.map((m) => '<tr><td class="mono">' + m.repo + '</td><td>' + pill('on uat', 'ok') + '</td><td class="mono faint small">' + m.commit + '</td><td class="faint small">' + esc(m.at) + '</td></tr>').join('') + '</tbody></table></div>' + (t.newCommits ? '<div class="warn-box">⚠ New commits on the ticket branches are not on uat. Activate the ticket again to re-merge.</div>' : '');
  } else step1 = '<div class="mute">Activate and test the ticket first. Integration needs the ticket to be active.</div>';

  const fails = anyFailed(t);
  const step2 = t.merges.length ? '<div class="tbwrap"><table class="tbl"><tbody>' + t.merges.map((m) => '<tr><td class="mono">' + m.repo + '</td><td>' + pipeChip(t, m.repo) + '</td><td>' + (t.deploy[m.repo].state === 'failed' ? btn('Re-run…', 'rerunAsk', { key: t.key, repo: m.repo }, 'sm') : '') + '</td></tr>').join('') + '</tbody></table></div><div class="faint small">Pipelines advance on their own (fake, a few seconds each). Use Simulate to make one fail.</div>' : '<div class="mute">Pipelines appear after the push.</div>';

  const d = draftOf(t, 'comment'); const dep = allDeployed(t);
  const step3 = d ? (d.status === 'posted' ? '<div class="card"><pre class="code">' + esc(d.body) + '</pre><div class="small">' + pill('posted to Jira', 'ok') + '</div></div>' : '<textarea rows="4" class="mono" data-bind="draft:' + t.key + ':' + d.id + '">' + esc(d.body) + '</textarea><div class="row">' + btn('Post to Jira…', 'postAsk', { key: t.key, draft: d.id }, 'p') + '<span class="faint small">Edit freely. Lists each repo with its PR and pipeline.</span></div>')
    : dep ? '<div class="row">' + btn('Draft the comment', 'composeDraft', { key: t.key }, 'p') + '</div>' : '<div class="mute">Available when every touched repo is deployed.</div>';
  const step4 = d && d.status === 'posted' ? (t.jira === JIRA.alpha || isSignedOff(t) ? '<div>' + pill(t.jira, jiraCls(t.jira)) + '</div>' : '<div class="row"><span>' + JIRA.review + ' → ' + JIRA.alpha + '</span>' + btn('Transition…', 'transitionAsk', { key: t.key }, 'p') + '</div>') : '<div class="mute">After the comment is posted.</div>';
  const pending = t.prs.filter((p) => !p.reviewers.find((r) => r.name === ME).approved);
  const step5 = t.merges.length ? '<div class="mute">Alpha and UAT are done by others. When Jira reaches a signed-off status (' + JIRA.signed.join(', ') + '), approval unlocks. New commits suggest a re-merge; a returned ticket only matters if you are mentioned.</div>' +
    (isSignedOff(t) ? '<div class="tbwrap"><table class="tbl"><tbody>' + t.prs.map((p) => { const ok = p.reviewers.find((r) => r.name === ME).approved; return '<tr><td class="mono">' + p.repo + ' #' + p.id + '</td><td>' + (ok ? pill('approved by you', 'ok') : pill('needs your approval', 'warn')) + '</td><td>' + (ok ? '' : btn('Approve…', 'approveAsk', { key: t.key, repo: p.repo, pr: p.id }, 'p sm')) + '</td></tr>'; }).join('') + '</tbody></table></div>' : '') : '<div class="mute">Later.</div>';
  const stepBox = (n, title, cls, body) => '<div class="step ' + cls + '"><div class="n">' + (cls === 'done' ? '✓' : n) + '</div><div class="sb2"><h3>' + title + '</h3>' + body + '</div></div>';
  return '<div class="steps">' + stepBox(1, 'Integrate to uat', s1, step1) + stepBox(2, 'Pipelines', t.merges.length ? (fails.length ? 'bad' : dep ? 'done' : 'now') : '', step2) + stepBox(3, 'Deploy comment', d ? (d.status === 'posted' ? 'done' : 'now') : dep ? 'now' : '', step3) + stepBox(4, 'Move ticket', d && d.status === 'posted' ? (t.jira === JIRA.alpha || isSignedOff(t) ? 'done' : 'now') : '', step4) + stepBox(5, 'After alpha', isSignedOff(t) ? 'now' : '', step5) + '</div>';
}
function tabTimeline(t) {
  const rows = S.audit.filter((a) => a.ticket === t.key);
  return '<section class="sec"><h3>Audit log for ' + t.key + '</h3><div class="faint small">Append-only. An “attempted” entry is written before any external write, the outcome after.</div>' + (rows.length ? '<div class="tbwrap"><table class="tbl"><thead><tr><th>Time</th><th>Action</th><th>Repo</th><th>Outcome</th><th>Details</th></tr></thead><tbody>' + rows.map(auditRow).join('') + '</tbody></table></div>' : '<div class="empty">No activity yet for this ticket.</div>') + '</section>';
}
function viewTicket() {
  const r = S.ui.route; const t = tk(r.key); if (!t) return '<div class="empty">Unknown ticket.</div>';
  const tab = r.tab || 'overview';
  return ticketHead(t, tab) + '<div class="tbody">' + ({ overview: tabOverview, review: tabReview, test: tabTest, ship: tabShip, timeline: tabTimeline }[tab] || tabOverview)(t) + '</div>';
}

/* ---------------- what is on uat ---------------- */
function viewEnv() {
  const cols = Object.keys(REPOS).filter((r) => r !== 'docs' || S.tickets.some((t) => t.merges.some((m) => m.repo === 'docs')));
  const per = (repo) => S.tickets.filter((t) => t.merges.some((m) => m.repo === repo)).map((t) => ({ t, m: t.merges.find((m) => m.repo === repo), d: t.deploy[repo] }));
  const overlaps = [];
  for (const repo of cols) { const list = per(repo); for (let i = 0; i < list.length; i++) for (let j = i + 1; j < list.length; j++) { const a = list[i].t.prs.filter((p) => p.repo === repo).flatMap((p) => p.files.map((f) => f.path)); const b = list[j].t.prs.filter((p) => p.repo === repo).flatMap((p) => p.files.map((f) => f.path)); a.filter((f) => b.includes(f)).forEach((f) => overlaps.push({ repo, a: list[i].t.key, b: list[j].t.key, file: f })); } }
  return '<h2>On uat</h2><p class="lede">uat accumulates tickets, so what is deployed to alpha is the sum of everything merged. Check this before pushing: two tickets touching the same file can behave differently together.</p>' +
    '<div class="colsgrid">' + cols.map((repo) => { const list = per(repo); return '<section class="envcol"><div class="row sb"><h3 class="mono">' + repo + '</h3><span class="pill">' + REPOS[repo].host + '</span></div>' + (list.length ? list.map(({ t, m, d }) => '<div class="envit"><div class="row sb"><a href="#" class="key" data-act="go" data-name="ticket" data-key="' + t.key + '" data-tab="ship">' + t.key + '</a><span class="mono faint small">' + m.commit + '</span></div><div class="small">' + esc(t.title) + '</div><div class="row small">' + (d ? pipeChip(t, repo) : '') + ' ' + pill(t.jira, jiraCls(t.jira)) + '</div></div>').join('') : '<div class="faint">Nothing merged by you.</div>') + '</section>'; }).join('') + '</div>' +
    '<section class="sec"><h3>Potential overlap</h3>' + (overlaps.length ? overlaps.map((o) => '<div class="warn-box"><b>' + o.repo + '</b>: ' + o.a + ' and ' + o.b + ' both change <span class="mono">' + esc(o.file) + '</span>.</div>').join('') : '<div class="faint">No overlapping files between tickets on uat.</div>') + '<div class="faint small">Idea: warn about this before a ticket is prepared, not after.</div></section>';
}

/* ---------------- workspace ---------------- */
function viewWorkspace() {
  const act = activeTicket();
  return '<div class="row sb"><h2>Workspace shop</h2><span class="row">' + pill(S.ws.up ? '● services up' : '○ services down', S.ws.up ? 'ok' : '') + (S.ws.up ? btn('Stop…', 'stopAsk', {}) : btn('Start', 'startWs', {}, 'p')) + '</span></div><p class="lede">Start and stop keep the CLI meaning: dependency order, and a guard for uncommitted or unpushed work.</p><div class="mono mute">db → api → web</div>' +
    '<div class="tbwrap"><table class="tbl"><thead><tr><th>Project</th><th>Services</th><th>Branch</th><th>State</th></tr></thead><tbody>' + Object.keys(REPOS).map((r) => { const ov = act && act.act && act.act.overlay && act.act.overlay.repo === r; const dirty = S.ws.dirty[r]; return '<tr><td class="mono">' + r + '</td><td>' + REPOS[r].services + '</td><td class="mono">' + esc(S.ws.branches[r]) + '</td><td>' + (S.ws.up ? pill('up', 'ok') : pill('down')) + ' ' + (dirty ? pill('uncommitted changes', 'warn') : pill('clean')) + (ov ? ' ' + pill('⚡ overlay', 'warn') : '') + '</td></tr>'; }).join('') + '</tbody></table></div>' +
    (act ? '<div class="card overlay">Active ticket <b>' + act.key + '</b> holds the repos above. Stashed local changes are restored when you park or finish.</div>' : '');
}

/* ---------------- audit ---------------- */
function auditRow(a) {
  return '<tr><td class="mono">' + a.at + '</td><td class="mono">' + esc(a.action) + '</td><td class="mono">' + esc(a.repo || '') + '</td><td>' + pill(a.outcome, a.outcome === 'success' ? 'ok' : a.outcome === 'failure' ? 'bad' : '') + '</td><td class="small">' + (a.ticket ? '<span class="key">' + a.ticket + '</span> ' : '') + esc(a.details) + '</td></tr>';
}
function viewAudit() {
  return '<h2>Audit log</h2><p class="lede">Every external write is recorded before it runs (attempted) and after (outcome). Read-only.</p>' + (S.audit.length ? '<div class="tbwrap"><table class="tbl"><thead><tr><th>Time</th><th>Action</th><th>Repo</th><th>Outcome</th><th>Details</th></tr></thead><tbody>' + S.audit.map(auditRow).join('') + '</tbody></table></div>' : '<div class="empty">Nothing yet. Do something.</div>');
}

/* ---------------- settings ---------------- */
function viewSettings() {
  const prov = (name, key, cmd, ver) => '<div class="card"><div class="row sb"><b>' + name + '</b>' + pill(S.providers[key].ready ? 'ready' : 'signed out', S.providers[key].ready ? 'ok' : 'warn') + '</div><div class="mute mono small">' + ver + '</div>' + (S.providers[key].ready ? '<div class="small mute">Logged in. Tokens live in the tool, never here.</div>' + btn('Simulate sign-out', 'provider', { k: key, v: 0 }, 'sm') : '<div class="small">Run <span class="mono">' + cmd + '</span> in a terminal, then re-check.</div>' + btn('Fake sign-in', 'provider', { k: key, v: 1 }, 'p sm')) + '</div>';
  return '<h2>Settings</h2><div class="grid3"><section class="sec"><h3>Providers</h3>' + prov('acli · Jira', 'jira', 'acli jira auth login', 'v1.3.39') + prov('bkt · Bitbucket', 'bkt', 'bkt auth login && bkt context use <name>', 'v0.32.1') + '<div class="row">' + btn('Re-check', 'recheck', {}) + btn('Diagnose…', 'diagnose', {}) + '</div><div class="faint small">Diagnose runs the read-only probe and shows raw tool output. It can contain ticket titles: read it before sharing.</div></section>' +
    '<section class="sec"><h3>Jira mapping</h3>' + kv('Review status', '<span class="in ro">' + JIRA.review + '</span>') + kv('Alpha status', '<span class="in ro">' + JIRA.alpha + '</span>') + kv('Returned', '<span class="in ro">' + JIRA.returned + '</span>') + kv('Signed off', '<span class="in ro">' + JIRA.signed.join(', ') + '</span>') + kv('Review JQL', '<span class="in ro">project = PROJ AND status = "In Review"</span>') + kv('My account id', '<span class="in ro">acct-0001</span>') + '<div class="faint small">Approval is only suggested once a ticket reaches a signed-off status.</div></section>' +
    '<section class="sec"><h3>Repos</h3><div class="tbwrap"><table class="tbl"><thead><tr><th>Repo</th><th>Base</th><th>Prod</th><th>Overlay</th></tr></thead><tbody>' + Object.entries(REPOS).map(([r, c]) => '<tr><td class="mono">' + r + '</td><td class="mono">' + c.base + '</td><td class="mono">' + c.prod + '</td><td>' + (c.overlayConsumer ? 'consumes ' + c.consumes : '–') + '</td></tr>').join('') + '</tbody></table></div><h3>Data</h3>' + kv('state.db', '<span class="mono">…/de/state.db</span>') + kv('cache.db', '<span class="mono">…/de/cache.db</span> ' + btn('reset cache', 'resetCache', {}, 'sm')) + '<div class="row">' + btn('Reset this demo', 'resetDemo', {}, 'sm') + '</div></section></div>';
}

/* ---------------- change ideas ---------------- */
const IDEAS = [
  { built: true, t: 'One progress stepper on every ticket screen', w: 'The state of a ticket (claimed, reviewed, testing, on uat, deployed, announced, alpha, signed off, approved) is the thing you most often need and today lives across three tabs. A persistent stepper under the ticket title makes it visible everywhere.' },
  { built: true, t: 'A “Now” bar for the active ticket, with the overlay warning', w: 'Only one ticket is active and its overlay must never reach uat. Keeping the active ticket, its timer and the overlay state pinned under the top bar on every screen removes the chance of forgetting.' },
  { built: true, t: 'An “On uat” screen: what is on alpha right now, per repo', w: 'uat accumulates tickets, so alpha is the sum of everything merged. A per-repo column of merged tickets with pipeline state answers “what am I about to add to?” and surfaces overlapping files.' },
  { built: true, t: '“What changed since I last looked” on tickets', w: 'As reviewer you re-open tickets constantly. New comments, new commits and mentions are counted per ticket and highlighted when you open it, instead of you diffing your memory.' },
  { built: true, t: 'Right sidebar for facts, left sidebar for places', w: 'Jira facts (fields, links, PRs, activity) change with the selected ticket; navigation does not. Keeping them in separate fixed columns stops the main area from carrying both.' },
  { built: true, t: 'Overlap warning before pushing, not after', w: 'When you prepare an integration, warn if another ticket already on uat changes the same file. It is not a conflict, but it is the case where alpha behaves differently from your local test.' },
  { built: false, t: 'Conflict pre-check while reviewing', w: 'Run the uat merge dry-run as soon as a ticket is reviewed, so a conflict is known before you spend time testing it locally.' },
  { built: false, t: 'Board view for the Review pool', w: 'A kanban option (Pool, In review, Awaiting alpha, Returned) for people who think in columns. The list stays the default because the ranking is the point.' },
  { built: false, t: 'Split diff and per-hunk “viewed”', w: 'Unified is built. Large PRs benefit from split view and marking individual hunks as viewed, not just files.' },
  { built: false, t: 'Per-repo review, one combined tree', w: 'Today each PR is a chip. One file tree grouped by repo would let you scan a two-repo ticket in one pass.' },
  { built: false, t: 'Automatic time tracking suggestions', w: 'Offer to start and stop the timer from activation and park, and show a daily total, so time never depends on remembering.' },
  { built: false, t: 'Pre-push checklist per repo', w: 'Let each repo declare its manual local checks (tests, lint) as a checklist that must be ticked before the push button enables. Today the gate is manual and varies.' },
  { built: false, t: 'Hold-to-confirm as an alternative to typing the key', w: 'Typing the ticket key is safe but slow. A 1.5 second press-and-hold with a visible ring is faster and still deliberate. Worth testing both.' },
];
function viewIdeas() {
  return '<h2>Change ideas</h2><p class="lede">Proposals for changing the design from the first outline. Those marked built are already in this prototype so you can judge them.</p><div class="stack">' + IDEAS.map((i) => '<article class="card"><div class="row sb"><b>' + esc(i.t) + '</b>' + pill(i.built ? 'built in prototype' : 'proposed', i.built ? 'ok' : '') + '</div><div class="mute">' + esc(i.w) + '</div></article>').join('') + '</div>';
}

function mainView() {
  const n = S.ui.route.name;
  return ({ next: viewNext, tickets: viewTickets, ticket: viewTicket, env: viewEnv, workspace: viewWorkspace, audit: viewAudit, settings: viewSettings, ideas: viewIdeas }[n] || viewNext)();
}
