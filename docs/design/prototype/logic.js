'use strict';
/* ============================ fake core ============================
   A small in-memory stand-in for de-core: state, the lifecycle actions the real core exposes,
   fake sync, fake Actions runs, and a simplified copy of the next-action rules. No I/O, no network. */

const MS_PER_MIN = 4000;            // one simulated minute per 4 real seconds
const START_MIN = 9 * 60 + 40;      // the simulated day starts at 09:40

function newState() {
  return {
    ms: 0, seq: 100, run: 480,
    sync: { running: false, lastOk: -2 * MS_PER_MIN, lastAttempt: -2 * MS_PER_MIN, lastError: null, report: [], script: 0 },
    providers: { jira: { ready: true }, gh: { ready: true } },
    locks: {}, pending: {}, staleRepos: {}, repoFetch: {},
    sim: { holdLocks: typeof document !== 'undefined', offline: false, conflict: null, leak: null, uatMoved: false, failDeploy: null },
    ws: {
      branches: { 'api-client': 'develop', web: 'wip/my-experiment', worker: 'develop', docs: 'master' },
      dirty: { web: true },
      up: true,
    },
    settings: { prWaitMin: 30 },
    tickets: seedTickets(), audit: [], responses: {}, order: null,
    ui: { route: { name: 'next' }, sheet: null, toasts: [], dev: false, right: true, left: false, showAllSug: false, drafts: {}, prSel: {}, fileSel: {}, palette: null, viewed: {}, diffMode: 'unified', sinceMode: {}, cursor: null },
  };
}
let S = newState();
const script = syncScript();

/* ---------------------------- helpers ---------------------------- */
const tk = (k) => S.tickets.find((t) => t.key === k);
const status = (t) => (t.local ? t.local.status : null);
const activeTicket = () => S.tickets.find((t) => status(t) === 'active');
const clock = (at) => { const m = START_MIN + Math.floor((at == null ? S.ms : at) / MS_PER_MIN); return String(Math.floor(m / 60) % 24).padStart(2, '0') + ':' + String(m % 60).padStart(2, '0'); };
const minsAgo = (ms) => Math.max(0, Math.floor((S.ms - ms) / MS_PER_MIN));
const nextSeq = () => ++S.seq;
const hex = (s) => { let h = 5381; for (const c of String(s)) h = ((h * 33) ^ c.charCodeAt(0)) >>> 0; return h.toString(16).padStart(8, '0').slice(0, 7); };
const isHotfix = (t) => t.prs.some((p) => REPOS[p.repo].prod !== REPOS[p.repo].base && p.dst === REPOS[p.repo].prod);
const mentionsMe = (c) => c.body.some((b) => (b.t === 'p' && /@\[you\]/i.test(b.x)));
const maxN = (t) => t.comments.reduce((m, c) => Math.max(m, c.n), 0);
const unseenMentions = (t) => t.comments.filter((c) => c.n > t.seenN && c.who !== ME && mentionsMe(c));
const unseenCount = (t) => t.comments.filter((c) => c.n > t.seenN && c.who !== ME).length;
const durationMs = (t) => t.timeMs + (t.act ? S.ms - t.act.startedMs : 0);
const fmtDur = (ms) => { const m = Math.floor(ms / MS_PER_MIN); return m >= 60 ? Math.floor(m / 60) + 'h ' + String(m % 60).padStart(2, '0') + 'm' : m + 'm'; };
const isSignedOff = (t) => JIRA.signed.includes(t.jira);

function audit(action, ticket, repo, outcome, details) {
  S.audit.unshift({ id: nextSeq(), at: clock(), action, ticket: ticket || null, repo: repo || null, outcome: outcome || 'success', details: details || '' });
}
function toast(msg, kind, undo) { S.ui.toasts.push({ id: nextSeq(), msg, kind: kind || 'info', undo: undo || null, until: S.ms + (undo ? 8000 : 6000) }); }

/* ---------------------------- repo locks ----------------------------
   One writer per repo. Any operation that switches branches, stashes, merges, pushes or fetches takes an exclusive lock on every
   repo it touches, all or nothing, and never waits: if a repo is busy the action stops and says who holds it. Reads take no lock.
   Holds last as long as the operation (here: the progress sheet); a leftover git lock is "stale" and is only removed on request. */
const LOCK_MS = { activate: 400, park: 400, prepare: 1200, push: 1500 };
const LOCK_VERB = { activate: 'activating', park: 'parking', prepare: 'preparing the uat merge for', push: 'pushing to uat for' };
const lockOf = (repo) => { const h = S.locks[repo]; return h && (h.until == null || h.until > S.ms) ? h : null; };
function lockText(h) {
  if (h.kind === 'sync') return 'background sync is fetching';
  if (h.kind === 'external') return h.by + ' is running ' + h.op;
  return 'de is ' + (LOCK_VERB[h.op] || h.op) + (h.ticket ? ' ' + h.ticket : '');
}
function lockMsg(repo, h) {
  return h.kind === 'stale' ? repo + ' has a leftover .git/index.lock and no git process is running. Remove it to continue. Nothing was changed.'
    : repo + ' is busy: ' + lockText(h) + '. Nothing was changed.';
}
function lockBusy(repos, op, ticket) {
  for (const r of [...new Set(repos)].sort()) {
    const h = lockOf(r); if (!h) continue;
    audit('lock.denied', ticket, r, 'failure', op + ' refused: ' + lockText(h));
    return { ok: false, busy: { repo: r, holder: h }, errors: [lockMsg(r, h)], error: lockMsg(r, h) };
  }
  return null;
}
function lockHold(repos, op, ticket) {
  if (!S.sim.holdLocks) return;
  const rs = [...new Set(repos)]; const until = S.ms + (LOCK_MS[op] || 500) * Math.max(1, rs.length);
  rs.forEach((r) => { S.locks[r] = { by: 'de', kind: 'app', op, ticket, since: S.ms, until }; });
}
function breakLock(repo) {
  const h = S.locks[repo];
  if (!h || h.kind !== 'stale') return { ok: false, error: 'That lock is held by a running process, so it is not stale.' };
  delete S.locks[repo]; audit('lock.remove_stale', null, repo, 'success', '.git/index.lock removed after checking no git process runs'); return { ok: true };
}

/* which repos a ticket touches (has a ticket branch), which are ambiguous */
function repoPlan(t) {
  return Object.keys(REPOS).map((repo) => {
    const cands = t.cands[repo] || [];
    const excluded = !!t.excl[repo];
    const manual = t.link[repo] || null;
    const chosen = excluded ? null : manual || (cands.length === 1 ? cands[0] : null);
    return { repo, cands, excluded, manual, chosen, ambiguous: !excluded && !manual && cands.length > 1 };
  }).filter((p) => p.cands.length || p.excluded);
}
const touchedRepos = (t) => repoPlan(t).filter((p) => p.chosen);

/* ---------------------------- missing pull requests ---------------------------- */
/* A ticket in Review with no PR (or a touched repo without one) cannot be reviewed. The app waits a while, since developers
   sometimes open the PR late, then prepares a return comment. Sending it still goes through the confirm sheet. */
const prStage = (t) => t.jira === JIRA.review && !t.merges.length && [null, 'claimed', 'reviewing', 'parked'].includes(status(t));
function prGap(t) {
  if (!prStage(t)) return null;
  const miss = touchedRepos(t).filter((p) => !t.prs.some((x) => x.repo === p.repo)).map((p) => ({ repo: p.repo, branch: p.chosen }));
  if (!t.prs.length) return { none: true, repos: miss };
  return miss.length ? { none: false, repos: miss } : null;
}
const hasPrGap = (t) => !!prGap(t);
const gapText = (t) => { const g = prGap(t); return g.none ? (g.repos.length ? 'No pull request yet. ' + g.repos.map((r) => r.repo + ' has ' + r.branch).join(', ') + ' but no PR.' : 'No pull request and no ticket branch found.') : 'No pull request yet in ' + g.repos.map((r) => r.repo + ' (' + r.branch + ')').join(', ') + '.'; };
const gapError = (t) => (hasPrGap(t) ? gapText(t) + ' Review can start once it exists.' : null);
const prWaitLeft = (t) => (t.prWait ? t.prWait.since + t.prWait.mins * MS_PER_MIN - S.ms : null);
const prWaitExpired = (t) => hasPrGap(t) && !!t.prWait && prWaitLeft(t) <= 0;
function startPrWait(t) { if (hasPrGap(t) && !t.prWait) t.prWait = { since: S.ms, mins: S.settings.prWaitMin }; }
function extendPrWait(key, mins) {
  const t = tk(key); if (!t) return;
  if (!t.prWait || prWaitLeft(t) <= 0) t.prWait = { since: S.ms, mins }; else t.prWait.mins += mins;
  audit('pr_wait.extend', key, null, 'success', mins + ' min');
}
function returnBody(t) {
  const g = prGap(t);
  const lines = ['Returned to development: ' + (g && g.none ? 'no pull request was found for this ticket.' : 'some branches have no pull request.')];
  lines.push(g && g.repos.length ? 'Branches without a PR: ' + g.repos.map((r) => r.repo + ' (' + r.branch + ')').join(', ') + '.' : 'No ticket branch or pull request was found in the workspace repositories.');
  if (t.prWait) lines.push('Waited ' + t.prWait.mins + ' min for it to appear (since ' + clock(t.prWait.since) + ').');
  lines.push('Please open a pull request for each branch and move the ticket back to ' + JIRA.review + '.');
  return lines.join('\n');
}
/* comment on the ticket and move it to Returned; an active ticket is parked first so the repos are restored */
function sendReturn(key, body, note) {
  const t = tk(key);
  if (t.act) { const d = deactivate(key, 'parked'); if (!d.ok) return { ok: false, error: d.errors[0], busy: d.busy }; }
  t.comments.push({ n: maxN(t) + 1, who: ME, at: 'today ' + clock(), body: body.split('\n').map((x) => P(x)) }); t.seenN = maxN(t);
  t.jira = JIRA.returned; t.local = null; t.prWait = null; t.thWait = null; t.reviewed = false;
  audit('jira.comment', key, null, 'success', 'returned: ' + note); audit('jira.transition', key, null, 'success', JIRA.review + ' -> ' + JIRA.returned);
  return { ok: true };
}
function returnMissingPr(key) {
  const t = tk(key); if (!t || !hasPrGap(t)) return { ok: false, error: 'A pull request exists now. Nothing was sent.' };
  return sendReturn(key, returnBody(t), 'no pull request');
}

/* ---------------------------- unresolved review comments ---------------------------- */
const threadStage = (t) => ['claimed', 'reviewing', 'parked'].includes(status(t)) && t.jira === JIRA.review && !t.merges.length;
const openThreads = (t) => t.prs.flatMap((p) => p.threads.filter((th) => !th.resolved).map((th) => ({ repo: p.repo, pr: p.id, th })));
const blockingThreads = (t) => (threadStage(t) ? openThreads(t).filter((x) => !(t.accepted || []).includes(x.th.id)) : []);
const acceptedThreads = (t) => openThreads(t).filter((x) => (t.accepted || []).includes(x.th.id));
const threadLine = (x) => x.repo + ' ' + x.th.file + (x.th.line ? ':' + x.th.line.slice(1) : '') + ' (' + x.th.author + '): ' + x.th.text;
const thWaitLeft = (t) => (t.thWait ? t.thWait.since + t.thWait.mins * MS_PER_MIN - S.ms : null);
const thWaitExpired = (t) => blockingThreads(t).length > 0 && !!t.thWait && thWaitLeft(t) <= 0;
function startThWait(t, hadBlocking) { if (!blockingThreads(t).length) return; if (!hadBlocking || !t.thWait) t.thWait = { since: S.ms, mins: S.settings.prWaitMin }; }
function extendThWait(key, mins) {
  const t = tk(key); if (!t) return;
  if (!t.thWait || thWaitLeft(t) <= 0) t.thWait = { since: S.ms, mins }; else t.thWait.mins += mins;
  audit('thread_wait.extend', key, null, 'success', mins + ' min');
}
function returnThreadsBody(t) {
  const th = blockingThreads(t);
  const lines = ['Returned to development: ' + th.length + ' review comment' + (th.length > 1 ? 's are' : ' is') + ' still unresolved.'];
  th.forEach((x) => lines.push('• ' + threadLine(x)));
  if (t.thWait) lines.push('Waited ' + t.thWait.mins + ' min for ' + (th.length > 1 ? 'them' : 'it') + ' to be resolved (since ' + clock(t.thWait.since) + ').');
  lines.push('Please resolve or answer them, then move the ticket back to ' + JIRA.review + '.');
  return lines.join('\n');
}
function returnThreads(key) {
  const t = tk(key); if (!t || !blockingThreads(t).length) return { ok: false, error: 'Nothing is unresolved now. Nothing was sent.' };
  return sendReturn(key, returnThreadsBody(t), 'unresolved review comments');
}
function acceptThreads(key) {
  const t = tk(key); const th = blockingThreads(t); t.accepted = (t.accepted || []).concat(th.map((x) => x.th.id));
  audit('threads.accept', key, null, 'success', th.length + ' unresolved comment' + (th.length > 1 ? 's' : '') + ' accepted'); return { ok: true, n: th.length };
}

/* ---------------------------- merge conflict with uat ---------------------------- */
const conflictSig = (t) => uatConflicts(t).map((c) => c.repo + ':' + c.with).join(',');
const conflictSent = (t) => !!t.conflictSent && t.conflictSent.sig === conflictSig(t);
function conflictBody(t) {
  const lines = [t.key + ' does not merge into uat: it conflicts with another ticket that is already there.'];
  uatConflicts(t).forEach((c) => lines.push('• ' + c.repo + ': conflicts with ' + c.with + ' in ' + c.files.join(', ')));
  lines.push('Please rebase the ticket branch on uat, or sort it out with the owner of ' + [...new Set(uatConflicts(t).map((c) => c.with))].join(', ') + '.');
  return lines.join('\n');
}
function sendConflict(key, alsoReturn) {
  const t = tk(key); if (!uatConflicts(t).length) return { ok: false, error: 'No conflict with uat now. Nothing was sent.' };
  const body = conflictBody(t); const sig = conflictSig(t);
  if (alsoReturn) { const r = sendReturn(key, body, 'merge conflict with uat'); return r.ok ? { ok: true, returned: true } : r; }
  t.comments.push({ n: maxN(t) + 1, who: ME, at: 'today ' + clock(), body: body.split('\n').map((x) => P(x)) }); t.seenN = maxN(t);
  t.conflictSent = { sig, at: clock() }; audit('jira.comment', key, null, 'success', 'merge conflict with uat'); return { ok: true, returned: false };
}

/* ---------------------------- local lifecycle ---------------------------- *//* ---------------------------- local lifecycle ---------------------------- */
/* one ticket in hand at a time: claimed, in review or active. Parked and awaiting-alpha tickets do not count. */
const inHand = (except) => S.tickets.find((x) => x.key !== except && ['claimed', 'reviewing', 'active'].includes(status(x)));
function parkInHand(key) {
  const t = tk(key); if (!t) return { ok: false };
  if (t.act) return deactivate(key, 'parked');
  t.local.status = 'parked'; audit('ticket.park', key, null, 'success', 'parked to take another ticket'); return { ok: true };
}
function claim(key) {
  const t = tk(key); if (!t || t.local) return { ok: false };
  const b = inHand(key); if (b) return { ok: false, blocker: b.key };
  t.local = { status: 'claimed' }; startPrWait(t); audit('ticket.claim', key); return { ok: true };
}
function startReview(key) {
  const t = tk(key); if (!t) return { ok: false };
  if (!t.local && inHand(key)) return { ok: false, blocker: inHand(key).key, error: 'Finish or park ' + inHand(key).key + ' first. One ticket at a time.' };
  if (gapError(t)) return { ok: false, error: gapError(t) };
  if (!t.local) t.local = { status: 'claimed' };
  if (['claimed', 'parked'].includes(status(t)) || status(t) === 'reviewing') t.local.status = status(t) === 'parked' ? 'parked' : 'reviewing';
  t.reviewed = false; t.reviewedSeq = null; audit('review.start', key); return { ok: true };
}
function markReviewed(key) {
  const t = tk(key); if (!t) return { ok: false };
  if (gapError(t)) return { ok: false, error: gapError(t) };
  t.reviewed = true; t.reviewedSeq = Math.max(...t.prs.map((p) => p.updatedSeq), 0); t.prs.forEach((p) => { p.since = null; });
  if (status(t) === 'claimed') t.local.status = 'reviewing';
  startThWait(t, false);
  audit('review.mark_reviewed', key); return { ok: true };
}
function toggleChecklist(key, i) { const t = tk(key); t.checklist[i].done = !t.checklist[i].done; }
function addChecklist(key, text) { const t = tk(key); if (text && text.trim()) t.checklist.push({ text: text.trim(), done: false }); }
function setLink(key, repo, branch) { const t = tk(key); t.link[repo] = branch; audit('links.choose', key, repo, 'success', branch); }
function markSeen(key) { const t = tk(key); if (t) t.seenN = maxN(t); }
function reclaim(key) {
  const t = tk(key); if (inHand(key)) return { ok: false, blocker: inHand(key).key }; t.local = { status: 'claimed' }; t.reviewed = false; t.reviewedSeq = null; t.merges = []; t.deploy = {}; t.drafts = []; t.newCommits = false; t.prep = null; t.pre = {}; t.prWait = null; t.thWait = null; t.accepted = []; t.conflictSent = null;
  audit('ticket.reclaim', key, null, 'success', 'back in Review, treated as new'); return { ok: true };
}
/* local-only actions can be undone for a few seconds; anything that sends to a remote goes through the confirm sheet instead */
function unclaim(key) { const t = tk(key); if (t && status(t) === 'claimed') { t.local = null; audit('undo.claim', key); } }
function unmarkReviewed(key, prev) { const t = tk(key); t.reviewed = false; t.reviewedSeq = prev.seq; t.local.status = prev.status; audit('undo.mark_reviewed', key); }
function untrackDone(key) { const t = tk(key); t.local = { status: 'done' }; audit('ticket.done', key); }

/* ---------------------------- activation ---------------------------- */
const baselineFor = (repo, hotfix, choice) => {
  if (!hotfix) return REPOS[repo].base;
  return choice === 'production' ? REPOS[repo].prod : choice === 'uat' ? 'uat' : REPOS[repo].base;
};
function activationPlan(t, choice) {
  const errors = []; const rows = [];
  if (gapError(t)) errors.push(gapError(t));
  const plans = Object.fromEntries(repoPlan(t).map((p) => [p.repo, p]));
  for (const repo of Object.keys(REPOS)) {
    const p = plans[repo];
    if (p && p.ambiguous) errors.push(repo + ': ' + p.cands.length + ' branches match ' + t.key + '. Choose one on the Overview tab.');
    if (p && p.chosen) rows.push({ repo, role: 'ticket', branch: p.chosen });
    else rows.push({ repo, role: 'baseline', branch: baselineFor(repo, isHotfix(t), choice) });
  }
  const overlayRow = rows.find((r) => REPOS[r.repo].overlayConsumer && rows.find((x) => x.repo === REPOS[r.repo].consumes && x.role === 'ticket'));
  const overlay = overlayRow ? { repo: overlayRow.repo, provider: REPOS[overlayRow.repo].consumes } : null;
  if (blockingThreads(t).length) errors.push(blockingThreads(t).length + ' unresolved review comment' + (blockingThreads(t).length > 1 ? 's' : '') + '. Wait for them to be resolved, return the ticket, or proceed anyway from the ticket page.');
  return { errors, rows, overlay };
}
function activate(key, choice) {
  const t = tk(key);
  if (!t || !t.local) return { ok: false, errors: ['Claim the ticket first.'] };
  const other = activeTicket();
  if (other) return { ok: false, errors: [other.key + ' is already active. Park or finish it first.'] };
  if (!['claimed', 'reviewing', 'parked', 'integrated'].includes(status(t))) return { ok: false, errors: ['A ' + status(t) + ' ticket cannot be activated.'] };
  if (isHotfix(t) && !choice) return { ok: false, needBaseline: true };
  const plan = activationPlan(t, choice);
  if (plan.errors.length) return { ok: false, errors: plan.errors };
  const busy = lockBusy(plan.rows.map((r) => r.repo), 'activate', key); if (busy) return busy;
  audit('activation.start', key);
  const records = []; const warnings = [];
  for (const r of plan.rows) {
    const prev = S.ws.branches[r.repo]; const stash = !!S.ws.dirty[r.repo];
    S.ws.branches[r.repo] = r.branch; if (stash) S.ws.dirty[r.repo] = false;
    records.push({ repo: r.repo, role: r.role, branch: r.branch, prev, stash, label: stash ? 'de:' + key + ':' + r.repo : null });
    audit('activation.repo_switched', key, r.repo, 'success', (r.role === 'ticket' ? 'ticket branch ' : 'baseline ') + r.branch);
  }
  if (plan.overlay) audit('overlay.apply', key, plan.overlay.repo, 'success', 'composer path repository -> ../' + plan.overlay.provider + ', constraint "*"');
  t.act = { startedMs: S.ms, records, overlay: plan.overlay, choice: choice || null };
  t.local.status = 'active'; t.prep = null; t.testFromN = maxN(t);
  audit('activation.complete', key); lockHold(plan.rows.map((r) => r.repo), 'activate', key);
  return { ok: true, rows: plan.rows, records, overlay: plan.overlay, warnings };
}
function deactivate(key, target, by) {
  const t = tk(key);
  if (!t || !t.act) return { ok: false, errors: ['Nothing to restore for ' + key + '.'] };
  if (by !== 'finalize') { const busy = lockBusy(t.act.records.map((r) => r.repo), 'park', key); if (busy) return busy; }
  const lockedRepos = t.act.records.map((r) => r.repo);
  const restored = []; let overlayReverted = null;
  if (t.act.overlay) { overlayReverted = t.act.overlay; audit('overlay.revert', key, t.act.overlay.repo, 'success', 'composer.json and composer.lock restored, composer install run'); }
  for (const r of [...t.act.records].reverse()) {
    S.ws.branches[r.repo] = r.prev; if (r.stash) S.ws.dirty[r.repo] = true;
    restored.push({ repo: r.repo, to: r.prev, stash: r.label });
    audit('deactivation.repo_restored', key, r.repo, 'success', 'back on ' + r.prev + (r.stash ? ', stash popped' : ''));
  }
  t.timeMs += S.ms - t.act.startedMs; t.act = null;
  t.local.status = target || 'parked';
  audit(by === 'finalize' ? 'integration.finalize' : 'deactivation.complete', key);
  if (by !== 'finalize') lockHold(lockedRepos, 'park', key);
  return { ok: true, restored, overlayReverted };
}

/* ---------------------------- uat pre-check (runs as soon as there are ticket branches) ---------------------------- */
function overlapsFor(t, repo) {
  const files = t.prs.filter((x) => x.repo === repo).flatMap((x) => x.files); const out = [];
  for (const o of S.tickets) {
    if (o.key === t.key || !o.merges.some((m) => m.repo === repo)) continue;
    for (const f of o.prs.filter((x) => x.repo === repo).flatMap((x) => x.files)) if (files.some((g) => g.path === f.path)) out.push({ key: o.key, file: f.path });
  }
  return out;
}
function uatCheck(t) {
  return touchedRepos(t).map((p) => {
    const repo = p.repo;
    if (t.merges.some((m) => m.repo === repo)) return { repo, state: 'onuat' };
    if (S.sim.conflict === repo) return { repo, state: 'conflict', with: 'PROJ-127', files: ['src/session.rs'] };
    const ov = overlapsFor(t, repo);
    return ov.length ? { repo, state: 'overlap', overlaps: ov } : { repo, state: 'clean' };
  });
}
const preMergeStage = (t) => ['claimed', 'reviewing', 'parked', 'active'].includes(status(t));
const uatConflicts = (t) => (preMergeStage(t) ? uatCheck(t).filter((r) => r.state === 'conflict') : []);
const uatOverlaps = (t) => (preMergeStage(t) ? uatCheck(t).filter((r) => r.state === 'overlap') : []);

/* ---------------------------- diff helpers ---------------------------- */
/* hunks: runs of changes with at most one context line between them; boundaries fall in the middle of longer context gaps */
function hunksOf(f) {
  const ch = []; f.lines.forEach((l, i) => { if (l.k !== 'ctx') ch.push(i); });
  if (!ch.length) return [{ a: 0, b: f.lines.length }];
  const groups = []; for (const c of ch) { const g = groups[groups.length - 1]; if (g && c - g.end <= 2) g.end = c; else groups.push({ start: c, end: c }); }
  const cuts = [0]; for (let i = 1; i < groups.length; i++) { const gap = groups[i].start - groups[i - 1].end - 1; cuts.push(groups[i - 1].end + 1 + Math.floor(gap / 2)); }
  cuts.push(f.lines.length);
  return groups.map((_, i) => ({ a: cuts[i], b: cuts[i + 1] }));
}
const hunkKey = (t, pr, f, i) => t.key + ':' + pr.id + ':' + f.path + ':' + i;
const viewedIn = (t, pr, f) => hunksOf(f).filter((_, i) => S.ui.viewed[hunkKey(t, pr, f, i)]).length;
/* pairs del/add runs for the split view */
function splitRows(lines) {
  const rows = []; let i = 0;
  while (i < lines.length) {
    if (lines[i].k === 'ctx') { rows.push({ l: lines[i], r: lines[i] }); i++; continue; }
    const dels = []; const adds = [];
    while (i < lines.length && lines[i].k === 'del') dels.push(lines[i++]);
    while (i < lines.length && lines[i].k === 'add') adds.push(lines[i++]);
    for (let j = 0; j < Math.max(dels.length, adds.length); j++) rows.push({ l: dels[j] || null, r: adds[j] || null });
  }
  return rows;
}
const staleReview = (t) => t.reviewed && t.prs.some((p) => p.updatedSeq > t.reviewedSeq);

/* ---------------------------- pre-push checks (per repo) ---------------------------- */
const prepushItems = (t) => touchedRepos(t).flatMap((p) => (REPOS[p.repo].checks || []).map((text, i) => ({ repo: p.repo, i, text, done: !!t.pre[p.repo + ':' + i] })));
const prepushMissing = (t) => prepushItems(t).filter((x) => !x.done);
function togglePre(key, repo, i) { const t = tk(key); t.pre[repo + ':' + i] = !t.pre[repo + ':' + i]; }

/* ---------------------------- integration ---------------------------- */
function prepare(key) {
  const t = tk(key);
  if (!t || status(t) !== 'active') return { ok: false, error: 'The ticket must be active (tested locally) before it can be integrated.' };
  const busy = lockBusy(touchedRepos(t).map((p) => p.repo), 'prepare', key); if (busy) return busy;
  const rows = touchedRepos(t).map((p) => {
    const repo = p.repo;
    const done = t.merges.find((m) => m.repo === repo);
    if (done) return { repo, outcome: 'alreadyPushed', commit: done.commit };
    if (S.sim.conflict === repo) return { repo, outcome: 'conflict', files: ['src/session.rs'], with: 'PROJ-127' };
    if (S.sim.leak === repo) return { repo, outcome: 'blocked', reason: 'overlay leak: composer.json adds a path repository (commit ' + hex('leak' + repo) + ')' };
    const files = t.prs.filter((x) => x.repo === repo).flatMap((x) => x.files);
    const overlaps = overlapsFor(t, repo);
    return { repo, outcome: 'ready', branch: p.chosen, commits: 2 + files.length, files: files.length, uatBefore: hex('u' + repo + S.seq), merge: hex('m' + repo + key + S.seq), overlaps };
  });
  t.prep = { at: S.ms, rows };
  audit('integration.prepare', key, null, rows.every((r) => r.outcome === 'ready' || r.outcome === 'alreadyPushed') ? 'success' : 'failure', rows.map((r) => r.repo + ':' + r.outcome).join(' '));
  touchedRepos(t).forEach((p) => { S.repoFetch[p.repo] = S.ms; delete S.staleRepos[p.repo]; });
  lockHold(touchedRepos(t).map((p) => p.repo), 'prepare', key);
  return { ok: true, prep: t.prep };
}
const prepReady = (t) => !!t.prep && t.prep.rows.some((r) => r.outcome === 'ready') && t.prep.rows.every((r) => r.outcome === 'ready' || r.outcome === 'alreadyPushed');
const prepPushable = (t) => prepReady(t) && prepushMissing(t).length === 0;
function push(key) {
  const t = tk(key); const prep = t && t.prep;
  if (!prep || !prepPushable(t)) return { ok: false, error: 'Nothing is ready to push. Prepare the integration again.' };
  const busy = lockBusy(prep.rows.map((r) => r.repo), 'push', key); if (busy) return busy;
  audit('git.push_uat.attempted', key, null, 'skipped', prep.rows.map((r) => r.repo + ' ' + (r.merge || r.commit) + ':refs/heads/uat').join('; '));
  const results = [];
  for (const r of prep.rows) {
    if (r.outcome === 'alreadyPushed') { results.push({ repo: r.repo, result: 'AlreadyPushed' }); continue; }
    if (S.sim.uatMoved) {
      results.push({ repo: r.repo, result: 'Rejected', reason: 'uat moved on the remote since you prepared' });
      audit('git.push_uat.repo', key, r.repo, 'failure', 'rejected: uat moved'); S.sim.uatMoved = false; t.prep = null; break;
    }
    t.merges.push({ repo: r.repo, commit: r.merge, at: clock() });
    t.deploy[r.repo] = { state: 'pending', run: ++S.run, since: S.ms, step: 'Queued' };
    results.push({ repo: r.repo, result: 'Pushed', commit: r.merge });
    audit('git.push_uat.repo', key, r.repo, 'success', r.uatBefore + ' -> ' + r.merge);
  }
  const allPushed = touchedRepos(t).every((p) => t.merges.some((m) => m.repo === p.repo));
  let finalized = false;
  if (allPushed && results.every((x) => x.result === 'Pushed' || x.result === 'AlreadyPushed')) {
    audit('git.push_uat', key, null, 'success', results.length + ' repos');
    deactivate(key, 'integrated', 'finalize'); t.prep = null; finalized = true;
  } else audit('git.push_uat', key, null, 'failure', 'not every repo was pushed');
  lockHold(prep.rows.map((r) => r.repo), 'push', key);
  return { ok: true, results, finalized };
}
function rerun(key, repo) {
  const t = tk(key); const d = t && t.deploy[repo]; if (!d) return { ok: false };
  d.run = ++S.run; d.state = 'pending'; d.since = S.ms; d.step = 'Queued'; d.log = null; if (S.sim.failDeploy === repo) S.sim.failDeploy = null;
  audit('github.actions_rerun', key, repo, 'success', 'new run #' + d.run); return { ok: true };
}
function stepDeploy() {
  for (const t of S.tickets) for (const [repo, d] of Object.entries(t.deploy)) {
    if (d.state === 'deployed' || d.state === 'failed') continue;
    const el = S.ms - d.since;
    if (el < 3000) { d.state = 'pending'; d.step = 'Queued'; }
    else if (el < 8000) { d.state = 'running'; d.step = el < 5000 ? 'Build' : el < 6500 ? 'Test' : 'Deploy alpha'; }
    else {
      d.state = S.sim.failDeploy === repo ? 'failed' : 'deployed'; d.step = 'Deploy alpha';
      if (d.state === 'failed') d.log = ['$ deploy --env alpha --repo ' + repo, 'build ok (41s)', 'tests ok (312 passed)', 'pushing image ' + REPOS[repo].host + ':' + hex('img' + d.run), 'rolling update: 1/3 ready', 'ERROR health check failed: GET /healthz returned 503 for 60s', 'step "Deploy alpha" exited with code 1'];
      toast(t.key + ' ' + repo + ' run #' + d.run + (d.state === 'failed' ? ' failed' : ' deployed to alpha'), d.state === 'failed' ? 'bad' : 'ok');
    }
  }
}
/* sync never skips a busy repo silently: it queues the fetch, runs it the moment the repo is free, and after the timeout marks the
   repo as not refreshed so everything derived from its branches and PRs carries a warning until a fetch succeeds */
const SYNC_WAIT_MS = 30000;
function stepPending() {
  for (const [r, p] of Object.entries(S.pending)) {
    if (!lockOf(r)) { S.repoFetch[r] = S.ms; delete S.staleRepos[r]; delete S.pending[r]; audit('sync.fetch_late', null, r, 'success', 'fetched after waiting ' + Math.round((S.ms - p.since) / 1000) + ' s'); continue; }
    if (S.ms >= p.deadline) {
      delete S.pending[r]; S.staleRepos[r] = { since: S.repoFetch[r] != null ? S.repoFetch[r] : p.since, by: p.by };
      S.sync.lastError = 'partial'; S.sync.report.push({ src: 'Git fetch', ok: false, text: r + ' not refreshed: busy for ' + SYNC_WAIT_MS / 1000 + ' s (' + p.by + '). Its data may be out of date. The next sync tries again' });
      audit('sync.fetch_timeout', null, r, 'failure', 'still busy: ' + p.by); toast(r + ' could not be refreshed: it stayed busy. Its data may be out of date.', 'warn');
    }
  }
}
function advance(ms) { S.ms += ms; stepDeploy(); stepPending(); S.ui.toasts = S.ui.toasts.filter((x) => x.until > S.ms); }

const allDeployed = (t) => t.merges.length > 0 && touchedRepos(t).every((p) => t.merges.some((m) => m.repo === p.repo)) && t.merges.every((m) => t.deploy[m.repo] && t.deploy[m.repo].state === 'deployed');
const anyFailed = (t) => t.merges.filter((m) => t.deploy[m.repo] && t.deploy[m.repo].state === 'failed');
const anyRunning = (t) => t.merges.some((m) => t.deploy[m.repo] && ['pending', 'running'].includes(t.deploy[m.repo].state));

/* ---------------------------- announce ---------------------------- */
const draftOf = (t, kind) => t.drafts.filter((d) => d.kind === kind).slice(-1)[0] || null;
const blockText = (b) => (b.t === 'ul' ? b.items.map((i) => '- ' + i).join('\n') : b.t === 'code' ? b.x : b.x).replace(/@\[([^\]]+)\]/g, '@$1');
function composeDraft(key) {
  const t = tk(key); if (!t || !allDeployed(t)) return { ok: false, error: 'Every touched repo must be deployed first.' };
  const lines = t.merges.map((m) => { const pr = t.prs.find((p) => p.repo === m.repo); const d = t.deploy[m.repo]; return '• ' + m.repo + '  PR #' + (pr ? pr.id : '-') + ' · run #' + d.run + ' · https://github.com/' + REPOS[m.repo].host + '/actions/runs/' + d.run; });
  const parts = ['Deployed to alpha:\n' + lines.join('\n')];
  if (t.checklist.length) parts.push('Tested locally:\n' + t.checklist.map((c) => (c.done ? '☑ ' : '☐ ') + c.text).join('\n'));
  if (acceptedThreads(t).length) parts.push('Proceeded with unresolved review comments:\n' + acceptedThreads(t).map((x) => '• ' + threadLine(x)).join('\n'));
  const during = t.testFromN == null ? [] : t.comments.filter((c) => c.n > t.testFromN);
  if (during.length) parts.push('Comments while testing:\n' + during.map((c) => '> ' + c.who + ' (' + c.at + '): ' + c.body.map(blockText).join(' ').replace(/\n/g, ' ')).join('\n'));
  t.drafts.push({ id: 'd' + nextSeq(), kind: 'comment', body: parts.join('\n\n'), status: 'draft' });
  audit('draft.compose', key); return { ok: true };
}
function postComment(key, draftId) {
  const t = tk(key); const d = t.drafts.find((x) => x.id === draftId);
  if (!d || d.status === 'posted') return { ok: false, error: 'That draft was already posted.' };
  d.status = 'posted'; t.comments.push({ n: maxN(t) + 1, who: ME, at: 'today ' + clock(), body: d.body.split('\n').map((x) => P(x)) });
  t.seenN = maxN(t); audit('jira.comment', key, null, 'success', 'deploy comment'); return { ok: true };
}
function transition(key) {
  const t = tk(key); const d = draftOf(t, 'comment');
  if (!d || d.status !== 'posted') return { ok: false, error: 'Post the deploy comment first.' };
  t.jira = JIRA.alpha; audit('jira.transition', key, null, 'success', JIRA.review + ' -> ' + JIRA.alpha); return { ok: true };
}
function postAndMove(key, draftId) {
  const t = tk(key); const r = postComment(key, draftId); if (!r.ok) return r;
  if (t.jira === JIRA.review) return transition(key);
  return r;
}
function postFreeComment(key, text) {
  const t = tk(key); t.comments.push({ n: maxN(t) + 1, who: ME, at: 'today ' + clock(), body: text.split('\n').map((x) => P(x)) });
  t.seenN = maxN(t); audit('jira.comment', key, null, 'success', 'free comment'); return { ok: true };
}

/* ---------------------------- review writes ---------------------------- */
function approve(key, repo, prId) {
  const t = tk(key); const pr = t.prs.find((p) => p.repo === repo && p.id === prId); const me = pr.reviewers.find((r) => r.name === ME);
  me.approved = true; audit('github.pr_approve', key, repo, 'success', '#' + prId);
  if (t.prs.every((p) => p.reviewers.find((r) => r.name === ME).approved)) { t.local.status = 'done'; audit('ticket.done', key); }
  return { ok: true };
}
function approveAll(key) {
  const t = tk(key); const pend = t.prs.filter((p) => !p.reviewers.find((r) => r.name === ME).approved);
  pend.forEach((p) => approve(key, p.repo, p.id)); return { ok: true, n: pend.length };
}
function requestChanges(key, repo, prId, text) {
  const t = tk(key); const pr = t.prs.find((p) => p.repo === repo && p.id === prId);
  const had = blockingThreads(t).length;
  pr.threads.push({ id: 'th' + nextSeq(), file: pr.files[0].path, line: null, author: ME, text: 'Changes requested: ' + text, resolved: false, replies: [] }); startThWait(t, had > 0);
  audit('github.pr_request_changes', key, repo, 'success', '#' + prId); return { ok: true };
}
function addThread(key, prId, file, line, text) {
  const t = tk(key); const pr = t.prs.find((p) => p.id === prId);
  const had = blockingThreads(t).length;
  pr.threads.push({ id: 'th' + nextSeq(), file, line, author: ME, text, resolved: false, replies: [] }); startThWait(t, had > 0);
  audit('github.pr_comment', key, pr.repo, 'success', file + ':' + line); return { ok: true };
}

/* ---------------------------- fake sync ---------------------------- */
function syncStart() {
  if (S.sync.running) return { ok: false };
  S.sync.running = true; S.sync.lastAttempt = S.ms;
  if (S.sim.holdLocks) Object.keys(REPOS).forEach((r) => { if (!lockOf(r)) S.locks[r] = { by: 'sync', kind: 'sync', op: 'fetch', since: S.ms, until: null }; });
  return { ok: true };
}
function syncFinish() {
  const report = []; S.sync.running = false;
  Object.keys(S.locks).forEach((r) => { if (S.locks[r].kind === 'sync') delete S.locks[r]; });
  if (S.sim.offline) { S.sync.lastError = 'network unreachable'; S.sync.report = [{ src: 'Jira (acli)', ok: false, text: 'network unreachable, cache kept' }, { src: 'GitHub (gh)', ok: false, text: 'network unreachable, cache kept' }]; audit('sync', null, null, 'failure', 'offline'); return { ok: false, report: S.sync.report }; }
  report.push({ src: 'Jira (acli)', ok: true, text: 'pool refreshed' });
  if (S.providers.gh.ready) report.push({ src: 'GitHub (gh)', ok: true, text: 'PRs and Actions runs refreshed' });
  else report.push({ src: 'GitHub (gh)', ok: false, text: 'signed out, cache kept. Run gh auth login' });
  Object.keys(REPOS).forEach((r) => {
    const h = lockOf(r);
    if (!h) { S.repoFetch[r] = S.ms; delete S.staleRepos[r]; delete S.pending[r]; return; }
    if (!S.pending[r]) S.pending[r] = { since: S.ms, deadline: S.ms + SYNC_WAIT_MS, by: lockText(h) };
    report.push({ src: 'Git fetch', ok: true, text: r + ' is busy (' + lockText(h) + '). Fetches as soon as it is free, gives up after ' + SYNC_WAIT_MS / 1000 + ' s' });
  });
  const ev = script[S.sync.script]; if (ev) { ev.apply(S); S.sync.script++; report.push({ src: 'Jira (acli)', ok: true, text: ev.text }); toast(ev.text, 'info'); }
  S.sync.lastOk = S.ms; S.sync.lastError = report.some((r) => !r.ok) ? 'partial' : null; S.sync.report = report;
  audit('sync', null, null, S.sync.lastError ? 'failure' : 'success', report.map((r) => r.text).join('; '));
  return { ok: true, report };
}
function syncNow() { syncStart(); return syncFinish(); }

/* ---------------------------- simulate (dev panel) ---------------------------- */
const SIM = {
  mention(key) { const t = tk(key); t.comments.push({ n: maxN(t) + 1, who: 'Jane Doe', at: 'today ' + clock(), body: [P('@[you] can you look at this again? It still misbehaves on staging.')] }); toast('New comment mentioning you on ' + key, 'info'); },
  signOff(key) { const t = tk(key); t.jira = 'Done'; toast(key + ' moved to Done in Jira (signed off)', 'ok'); },
  returned(key) { const t = tk(key); t.jira = JIRA.returned; t.comments.push({ n: maxN(t) + 1, who: 'Jane Doe', at: 'today ' + clock(), body: [P('Sent back. @[you] please re-check.')] }); toast(key + ' was returned in Jira', 'warn'); },
  backToReview(key) { const t = tk(key); t.jira = JIRA.review; toast(key + ' is back in the Review column', 'info'); },
  extLock(repo) { if (S.locks[repo] && S.locks[repo].kind === 'external') delete S.locks[repo]; else S.locks[repo] = { by: 'de CLI', kind: 'external', op: 'de stop', since: S.ms, until: null }; },
  staleLock(repo) { if (S.locks[repo] && S.locks[repo].kind === 'stale') delete S.locks[repo]; else S.locks[repo] = { by: 'git', kind: 'stale', since: S.ms, until: null }; },
  prArrives(key) { const t = tk(key); openFakePrs(t); toast('A pull request was opened for ' + key, 'ok'); },
  resolveThreads(key) { const t = tk(key); t.prs.forEach((p) => p.threads.forEach((th) => { th.resolved = true; })); t.thWait = null; toast('The review comments on ' + key + ' were resolved', 'ok'); },
  skipMin(m) { advance(m * MS_PER_MIN); },
  uatAfterPush(key) { const t = tk(key); t.merges.forEach((m) => { const d = t.deploy[m.repo]; if (d) d.uatMoved = { by: 'Priya Nair', ticket: 'PROJ-150', at: clock() }; }); toast('Someone pushed to uat after ' + key, 'warn'); },
  newCommits(key) { const t = tk(key); t.prs.forEach((p) => { p.updatedSeq += 1; const f = p.files[0]; Object.keys(S.ui.viewed).forEach((k) => { if (k.startsWith(key + ':' + p.id + ':' + f.path + ':')) delete S.ui.viewed[k]; });
    p.since = { commit: hex('c' + p.id + p.updatedSeq), msg: 'Address review comments', files: [{ path: f.path, adds: 2, dels: 1, lines: [L('ctx', 30, 30, '// existing code'), L('del', 31, null, '// handle the empty case'), L('add', null, 31, '// address review: log and handle the empty case'), L('add', null, 32, '// covered by the new test'), L('ctx', 32, 33, '// existing code')] }] }; }); t.newCommits = true; toast('New commits pushed to the branches of ' + key, 'info'); },
  toggle(name, val) { S.sim[name] = val; },
  gh(ready) { S.providers.gh.ready = ready; },
  jira(ready) { S.providers.jira.ready = ready; },
};

/* ============================ next-action rules ============================
   Simplified from the real engine: pure over S, ranked, each with a stated reason. */
const BANDS = { repo_stale: 400, stale_lock: 880, threads_return: 855, threads_waiting: 545, pr_missing_return: 860, pr_waiting: 550, uat_conflict: 900, uat_moved: 700, prepush_checks: 678, deploy_failed: 1000, integration_blocked: 990, returned_to_review: 850, returned_mention: 840, re_review: 830, start_review: 820, finish_review: 810,
  approve_prs: 690, integrate_ready: 680, push_ready: 675, compose_deploy_comment: 670, post_deploy_comment: 665, transition_alpha: 660, remerge_needed: 645,
  activate_reviewed: 640, park_active: 620, deploy_waiting: 600, claim_new: 480, adapter_unavailable: 299, sync_stale: 1200 };

function suggest() {
  const out = []; const act = activeTicket();
  const push_ = (s) => { out.push(Object.assign({ level: 'local', info: false }, s, { priority: (BANDS[s.rule] || 100) + (s.rank ? -s.rank * 10 : 0) })); };

  // sync
  const age = minsAgo(S.sync.lastOk); const sinceAttempt = minsAgo(S.sync.lastAttempt);
  if (!S.sync.running && ((S.sync.lastError && sinceAttempt >= 15) || (!S.sync.lastError && age >= 6 && sinceAttempt >= 1)))
    push_({ id: '-:sync_stale:all', ticket: null, rule: 'sync_stale', title: 'Sync now', reason: 'Data was last refreshed ' + age + ' min ago.', level: 'auto', act: { do: 'sync' }, hash: 'sync' });

  Object.entries(S.locks).filter(([, h]) => h.kind === 'stale').forEach(([repo, h]) => push_({ id: '-:stale_lock:' + repo, ticket: null, rule: 'stale_lock', title: 'Stale git lock in ' + repo, reason: repo + ' has a leftover .git/index.lock and no git process is running, so nothing can change in that repo. Remove the lock.', level: 'local', act: { do: 'breakLock', repo }, hash: 'sl' + h.since }));

  Object.entries(S.staleRepos).forEach(([repo, s]) => push_({ id: '-:repo_stale:' + repo, ticket: null, rule: 'repo_stale', title: 'Sync ' + repo + ' again', reason: repo + ' stayed busy during the last sync (' + s.by + '), so its branches and PR data may be out of date, including the uat check.', level: 'auto', act: { do: 'sync' }, hash: 'rs' + s.since }));

  for (const t of S.tickets) {
    const st = status(t); const hot = isHotfix(t); const key = t.key;
    const need = (rule, sug) => push_(Object.assign({ ticket: key, rule, hotfix: hot }, sug));

    // returned + mentioned, or any unseen mention
    const ms = unseenMentions(t);
    if (ms.length && ['integrated', 'done', null, 'claimed', 'reviewing', 'parked', 'active'].includes(st))
      need('returned_mention', { id: key + ':returned_mention', title: (t.jira === JIRA.returned ? 'Returned: you were mentioned' : 'You were mentioned'), reason: ms[ms.length - 1].who + ' mentioned you on ' + key + ' (' + ms.length + ' unseen comment' + (ms.length > 1 ? 's' : '') + ').', info: true, act: { do: 'open', key, tab: 'overview' }, hash: 'm' + ms.length });
    if (t.jira === JIRA.review && (st === 'integrated' || st === 'done') && !t.newCommits && draftOf(t, 'comment') && draftOf(t, 'comment').status === 'posted' && t.merges.length)
      need('returned_to_review', { id: key + ':returned_to_review', title: 'Back in the Review column', reason: key + ' returned to Review after alpha. Treat it as new: claim it again.', act: { do: 'reclaim', key }, hash: 'rr' });

    if (!conflictSent(t)) uatConflicts(t).forEach((c) => need('uat_conflict', { id: key + ':uat_conflict:' + c.repo, title: 'Conflict with uat in ' + c.repo, reason: key + ' will not merge into uat: it conflicts with ' + c.with + ' in ' + c.files.join(', ') + '. Send the developer a prepared comment, and return it if you like.', level: 'external', needs: 'acli', act: { do: 'conflictAsk', key }, hash: 'uc' + c.with }));
    if (prGap(t) && st) {
      const left = prWaitLeft(t);
      if (t.prWait && left <= 0) need('pr_missing_return', { id: key + ':pr_return', title: 'Return ' + key + ': no pull request after ' + t.prWait.mins + ' min', reason: gapText(t) + ' Review cannot start. Return it with a comment that explains.', level: 'external', needs: 'acli', act: { do: 'returnAsk', key }, hash: 'pr' + t.prWait.since + ':' + t.prWait.mins });
      else need('pr_waiting', { id: key + ':pr_waiting', title: 'Waiting for the pull request', reason: gapText(t) + ' ' + Math.ceil(left / MS_PER_MIN) + ' min left. It clears itself when the PR appears.', info: true, level: 'auto', act: { do: 'open', key, tab: 'overview' }, hash: 'pw' });
      continue;
    }
    const bt = blockingThreads(t);
    if (bt.length && t.reviewed && st) {
      const left = thWaitLeft(t);
      if (t.thWait && left <= 0) need('threads_return', { id: key + ':threads_return', title: 'Return ' + key + ': ' + bt.length + ' unresolved comment' + (bt.length > 1 ? 's' : '') + ' after ' + t.thWait.mins + ' min', reason: 'The review comments are still open, so activation is blocked. Return it with a comment that lists them, or proceed anyway.', level: 'external', needs: 'acli', act: { do: 'returnThreadsAsk', key }, alt: { do: 'acceptThreadsAsk', key, label: 'Proceed anyway…' }, hash: 'tr' + t.thWait.since + ':' + t.thWait.mins + ':' + bt.length });
      else need('threads_waiting', { id: key + ':threads_waiting', title: 'Waiting for ' + bt.length + ' comment' + (bt.length > 1 ? 's' : '') + ' to be resolved', reason: 'Activation is blocked until they are. ' + (t.thWait ? Math.ceil(left / MS_PER_MIN) + ' min left. ' : '') + 'It clears itself when they are resolved.', info: true, level: 'auto', act: { do: 'open', key, tab: 'overview' }, alt: { do: 'acceptThreadsAsk', key, label: 'Proceed anyway…' }, hash: 'tw' + bt.length });
      continue;
    }
    if (!st && t.jira === JIRA.review) continue;   // unclaimed handled by claim_new below
    if (st === 'claimed') need('start_review', { id: key + ':start_review', title: 'Start the review', reason: key + ' is claimed. Read its ' + t.prs.length + ' PR' + (t.prs.length > 1 ? 's' : '') + ' against ' + [...new Set(t.prs.map((p) => p.dst))].join(', ') + '.', act: { do: 'startReview', key }, hash: 's' });
    if (st === 'reviewing' && !t.reviewed) need('finish_review', { id: key + ':finish_review', title: 'Finish the review', reason: 'Review in progress. Mark it reviewed when the diffs and comments are done.', act: { do: 'open', key, tab: 'review' }, hash: 'f' });
    if (t.reviewed && t.prs.some((p) => p.updatedSeq > t.reviewedSeq) && ['reviewing', 'parked', 'claimed'].includes(st))
      need('re_review', { id: key + ':re_review', title: 'Re-review: new commits', reason: 'The branches moved since you reviewed ' + key + '.', act: { do: 'startReview', key }, hash: 'rv' + Math.max(...t.prs.map((p) => p.updatedSeq)) });

    if (!act && t.reviewed && ['reviewing', 'parked'].includes(st) && t.jira !== JIRA.returned)
      need('activate_reviewed', { id: key + ':activate', title: hot ? 'Activate hotfix (choose baseline)' : 'Activate for local test', reason: 'Reviewed' + (st === 'parked' ? ' and parked' : '') + '. Nothing is active. This switches the touched repos to the ticket branches and stashes local changes.', act: { do: 'activate', key }, hash: 'a' });
    if (act && act.key !== key && hot && t.reviewed && ['reviewing', 'parked'].includes(st))
      push_({ id: act.key + ':park_active:' + key, ticket: act.key, rule: 'park_active', hotfix: true, title: 'Park ' + act.key + ' for the hotfix ' + key, reason: key + ' is a hotfix waiting to be tested. Parking restores the repos and reverts the overlay.', act: { do: 'park', key: act.key }, hash: 'pa' });

    if (st === 'active') {
      const done = t.checklist.length > 0 && t.checklist.every((c) => c.done); const pushed = touchedRepos(t).every((p) => t.merges.some((m) => m.repo === p.repo));
      const prepped = t.prep && t.prep.rows.some((r) => r.outcome === 'conflict' || r.outcome === 'blocked');
      if (done && !t.prep && !pushed) need('integrate_ready', { id: key + ':integrate_ready', title: 'Prepare integration to uat', reason: 'Checklist complete (' + t.checklist.length + ' of ' + t.checklist.length + '). ' + touchedRepos(t).length + ' repo' + (touchedRepos(t).length > 1 ? 's' : '') + ' touched. Preparing pushes nothing.', act: { do: 'prepare', key }, hash: 'i' });
      if (prepReady(t) && prepushMissing(t).length) need('prepush_checks', { id: key + ':prepush', title: 'Tick the pre-push checks', reason: prepushMissing(t).length + ' check' + (prepushMissing(t).length > 1 ? 's' : '') + ' left before the push unlocks: ' + prepushMissing(t).map((x) => x.repo + ': ' + x.text).join(', ') + '.', act: { do: 'open', key, tab: 'ship' }, hash: 'pp' + prepushMissing(t).length });
      if (t.prep && prepPushable(t)) need('push_ready', { id: key + ':push_ready', title: 'Review and push to uat', reason: 'Merge ready in ' + t.prep.rows.filter((r) => r.outcome === 'ready').map((r) => r.repo).join(', ') + '. Pushing deploys to alpha.', level: 'external', act: { do: 'pushAsk', key }, hash: 'p' + t.prep.at });
      if (prepped) t.prep.rows.filter((r) => r.outcome === 'conflict' || r.outcome === 'blocked').forEach((r) => need('integration_blocked', { id: key + ':blocked:' + r.repo, title: r.outcome === 'conflict' ? 'Resolve the uat conflict in ' + r.repo : 'Integration blocked in ' + r.repo, reason: r.outcome === 'conflict' ? 'Merging into uat conflicts with ' + r.with + ' in ' + r.files.join(', ') + '. Nothing was pushed.' : r.reason, info: true, act: { do: 'open', key, tab: 'ship' }, hash: r.outcome + r.repo }));
    }

    // A returned ticket is only your business if you are mentioned (handled above) or it fails to deploy.
    const sentBack = t.jira === JIRA.returned;
    if (t.merges.length) {
      for (const m of anyFailed(t)) {
        const d = t.deploy[m.repo];
        need('deploy_failed', { id: key + ':deploy_failed:' + m.repo, title: 'Deploy failed in ' + m.repo, reason: 'The deploy step of run #' + d.run + ' failed after your push to uat.', info: false, level: 'external', needs: 'gh', act: { do: 'rerunAsk', key, repo: m.repo }, hash: 'df' + d.run });
      }
      if (sentBack) continue;
      const moved = t.merges.find((m) => t.deploy[m.repo] && t.deploy[m.repo].uatMoved);
      if (moved && !isSignedOff(t)) { const u = t.deploy[moved.repo].uatMoved; need('uat_moved', { id: key + ':uat_moved', title: 'uat moved after your push', reason: u.ticket + ' (' + u.by + ') was pushed to uat at ' + u.at + '. Alpha now has both, so check the combined behaviour.', info: true, act: { do: 'open', key, tab: 'ship' }, hash: 'um' + u.at }); }
      if (anyRunning(t) && !anyFailed(t).length) need('deploy_waiting', { id: key + ':deploy_waiting', title: 'Waiting for Actions runs', reason: t.merges.filter((m) => ['pending', 'running'].includes(t.deploy[m.repo].state)).map((m) => m.repo + ' #' + t.deploy[m.repo].run + ' (' + t.deploy[m.repo].step + ')').join(', ') + '.', info: true, level: 'auto', act: { do: 'open', key, tab: 'ship' }, hash: 'w' });
      const d = draftOf(t, 'comment');
      if (allDeployed(t) && !d) need('compose_deploy_comment', { id: key + ':compose', title: 'Draft the deploy comment', reason: 'Every touched repo is deployed to alpha. The draft lists each repo with its PR and Actions run.', act: { do: 'composeDraft', key }, hash: 'c' });
      if (d && d.status === 'draft') need('post_deploy_comment', { id: key + ':post', title: 'Post the deploy comment and move to ' + JIRA.alpha, reason: 'A draft is ready with the deploy, your checklist and the comments from testing. Edit it, then post; the ticket moves in the same step.', level: 'external', needs: 'acli', act: { do: 'postAsk', key, draft: d.id }, hash: 'pc' + d.id });
      if (d && d.status === 'posted' && t.jira === JIRA.review) need('transition_alpha', { id: key + ':transition', title: 'Move to ' + JIRA.alpha, reason: 'The comment is posted but the ticket did not move. Transition ' + JIRA.review + ' → ' + JIRA.alpha + '.', level: 'external', needs: 'acli', act: { do: 'transitionAsk', key }, hash: 't' });
    }
    if (st === 'integrated' && t.newCommits && !act) need('remerge_needed', { id: key + ':remerge', title: hot ? 'Re-merge needed (choose baseline)' : 'Re-merge needed', reason: 'The ticket branches have commits that uat lacks after your recorded merge. Activate it again, test, and integrate again.', act: { do: 'activate', key }, hash: 'rm' });
    if (st === 'integrated' && isSignedOff(t)) {
      const pend = t.prs.filter((p) => !p.reviewers.find((r) => r.name === ME).approved);
      if (pend.length > 1) need('approve_prs', { id: key + ':approve:all', title: 'Approve ' + pend.length + ' pull requests', reason: key + ' passed testing (Jira: ' + t.jira + '). ' + pend.map((p) => p.repo + ' #' + p.id).join(', ') + '. Approval is the last gate.', level: 'external', needs: 'gh', act: { do: 'approveAllAsk', key }, hash: 'apa' + pend.length });
      else pend.forEach((p) => need('approve_prs', { id: key + ':approve:' + p.repo + '#' + p.id, title: 'Approve ' + REPOS[p.repo].host + ' #' + p.id, reason: key + ' passed testing (Jira: ' + t.jira + '). Approval is the last gate.', level: 'external', needs: 'gh', act: { do: 'approveAsk', key, repo: p.repo, pr: p.id }, hash: 'ap' + p.id }));
    }
  }

  // claim queue
  const pool = S.tickets.filter((t) => !t.local && t.jira === JIRA.review).sort((a, b) => (PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority]) || a.key.localeCompare(b.key, undefined, { numeric: true }));
  pool.filter((t) => !inHand(t.key) || isHotfix(t)).forEach((t, i) => push_({ id: t.key + ':claim_new', ticket: t.key, rule: 'claim_new', hotfix: isHotfix(t), rank: i, title: 'Claim ' + t.key, reason: 'In the Review column, priority ' + t.priority + (isHotfix(t) ? ', targets production (hotfix)' : '') + (hasPrGap(t) ? ', but it has no pull request yet' : '') + ', #' + (i + 1) + ' in your queue.', act: { do: 'claim', key: t.key }, hash: 'q', facts: { queue: i } }));

  // hotfix boost, adapter availability
  const res = out.map((s) => {
    let p = s.priority; if (s.hotfix && s.priority >= 400 && s.rule !== 'park_active') p += 200;
    if (s.needs && !S.providers[s.needs === 'acli' ? 'jira' : 'gh'].ready) {
      const name = s.needs === 'acli' ? 'acli' : 'gh';
      return Object.assign({}, s, { id: '-:adapter:' + name + ':' + s.rule, ticket: s.ticket, rule: 'adapter_unavailable', title: name + ' is signed out', reason: 'Cannot ' + s.title.toLowerCase() + ' until ' + name + ' is logged in. Run ' + name + ' auth login.', info: true, level: 'auto', act: { do: 'settings' }, priority: 299, hash: 'ad' });
    }
    return Object.assign({}, s, { priority: p });
  });
  const seen = new Set(); const uniq = res.filter((s) => (seen.has(s.id) ? false : (seen.add(s.id), true)));
  uniq.sort((a, b) => (b.priority - a.priority) || a.id.localeCompare(b.id));
  uniq.forEach((s, i) => { s.rank = i + 1; s.state = sugState(s); });
  return uniq;
}
function sugState(s) {
  const r = S.responses[s.id]; if (!r) return 'open';
  if (r.kind === 'dismissed') return r.hash === s.hash ? 'dismissed' : 'resurfaced';
  if (r.kind === 'snoozed') return S.ms < r.until ? 'snoozed' : 'open';
  return 'open';
}
/* what needs you now: something is broken, someone is waiting, or a hotfix is in play */
const ATTENTION = new Set(['stale_lock', 'threads_return', 'pr_missing_return', 'uat_conflict', 'deploy_failed', 'integration_blocked', 'returned_mention', 'returned_to_review', 're_review', 'approve_prs', 'adapter_unavailable']);
const attention = () => visibleSuggestions().filter((s) => !s.info && (ATTENTION.has(s.rule) || s.hotfix)).sort((a, b) => b.priority - a.priority);
const visibleSuggestions = () => suggest().filter((s) => s.state === 'open' || s.state === 'resurfaced');
function dismiss(id) { const s = suggest().find((x) => x.id === id); S.responses[id] = { kind: 'dismissed', hash: s ? s.hash : '' }; audit('suggestion.dismiss', s && s.ticket, null, 'success', id); }
function snooze(id, mins) { S.responses[id] = { kind: 'snoozed', until: S.ms + mins * MS_PER_MIN }; audit('suggestion.snooze', null, null, 'success', id + ' ' + mins + 'm'); }

/* Test hook (node only) */
if (typeof window === 'undefined') {
  globalThis.__DE = { get S() { return S; }, reclaim, syncStart, syncFinish, reset() { S = newState(); script.length = 0; syncScript().forEach((e) => script.push(e)); }, tk, claim, startReview, markReviewed, toggleChecklist, activate, deactivate, prepare, push, rerun, advance, allDeployed,
    blockingThreads, acceptedThreads, thWaitLeft, thWaitExpired, extendThWait, returnThreadsBody, returnThreads, acceptThreads, conflictBody, sendConflict, conflictSent, prGap, hasPrGap, gapText, prWaitLeft, prWaitExpired, extendPrWait, returnBody, returnMissingPr, inHand, parkInHand, lockOf, lockBusy, breakLock, composeDraft, postComment, postAndMove, approveAll, unclaim, uatCheck, uatConflicts, uatOverlaps, hunksOf, splitRows, prepushItems, prepushMissing, togglePre, staleReview, transition, approve, syncNow, SIM, suggest, visibleSuggestions, attention, dismiss, snooze, setLink, markSeen, repoPlan, isHotfix, draftOf, postFreeComment, addThread, requestChanges, activeTicket, unseenMentions };
}
