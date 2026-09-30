'use strict';
/* ============================ fake core ============================
   A small in-memory stand-in for de-core: state, the lifecycle actions the real core exposes,
   fake sync, fake pipelines, and a simplified copy of the next-action rules. No I/O, no network. */

const MS_PER_MIN = 4000;            // one simulated minute per 4 real seconds
const START_MIN = 9 * 60 + 40;      // the simulated day starts at 09:40

function newState() {
  return {
    ms: 0, seq: 100, run: 480,
    sync: { running: false, lastOk: -2 * MS_PER_MIN, lastAttempt: -2 * MS_PER_MIN, lastError: null, report: [], script: 0 },
    providers: { jira: { ready: true }, bkt: { ready: true } },
    sim: { offline: false, conflict: null, leak: null, uatMoved: false, failDeploy: null },
    ws: {
      branches: { 'api-client': 'develop', web: 'wip/my-experiment', worker: 'develop', docs: 'master' },
      dirty: { web: true },
      up: true,
    },
    tickets: seedTickets(), audit: [], responses: {}, order: null,
    ui: { route: { name: 'next' }, sheet: null, toasts: [], dev: false, right: true, left: false, showAllSug: false, drafts: {}, prSel: {}, fileSel: {}, palette: null },
  };
}
let S = newState();
const script = syncScript();

/* ---------------------------- helpers ---------------------------- */
const tk = (k) => S.tickets.find((t) => t.key === k);
const status = (t) => (t.local ? t.local.status : null);
const activeTicket = () => S.tickets.find((t) => status(t) === 'active');
const clock = () => { const m = START_MIN + Math.floor(S.ms / MS_PER_MIN); return String(Math.floor(m / 60) % 24).padStart(2, '0') + ':' + String(m % 60).padStart(2, '0'); };
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
function toast(msg, kind) { S.ui.toasts.push({ id: nextSeq(), msg, kind: kind || 'info', until: S.ms + 6000 }); }

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

/* ---------------------------- local lifecycle ---------------------------- */
function claim(key) {
  const t = tk(key); if (!t || t.local) return { ok: false };
  t.local = { status: 'claimed' }; audit('ticket.claim', key); return { ok: true };
}
function startReview(key) {
  const t = tk(key); if (!t) return { ok: false };
  if (!t.local) t.local = { status: 'claimed' };
  if (['claimed', 'parked'].includes(status(t)) || status(t) === 'reviewing') t.local.status = status(t) === 'parked' ? 'parked' : 'reviewing';
  t.reviewed = false; t.reviewedSeq = null; audit('review.start', key); return { ok: true };
}
function markReviewed(key) {
  const t = tk(key); if (!t) return { ok: false };
  t.reviewed = true; t.reviewedSeq = Math.max(...t.prs.map((p) => p.updatedSeq), 0);
  if (status(t) === 'claimed') t.local.status = 'reviewing';
  audit('review.mark_reviewed', key); return { ok: true };
}
function toggleChecklist(key, i) { const t = tk(key); t.checklist[i].done = !t.checklist[i].done; }
function addChecklist(key, text) { const t = tk(key); if (text && text.trim()) t.checklist.push({ text: text.trim(), done: false }); }
function setLink(key, repo, branch) { const t = tk(key); t.link[repo] = branch; audit('links.choose', key, repo, 'success', branch); }
function markSeen(key) { const t = tk(key); if (t) t.seenN = maxN(t); }
function reclaim(key) {
  const t = tk(key); t.local = { status: 'claimed' }; t.reviewed = false; t.reviewedSeq = null; t.merges = []; t.deploy = {}; t.drafts = []; t.newCommits = false; t.prep = null;
  audit('ticket.reclaim', key, null, 'success', 'back in Review, treated as new'); return { ok: true };
}
function untrackDone(key) { const t = tk(key); t.local = { status: 'done' }; audit('ticket.done', key); }

/* ---------------------------- activation ---------------------------- */
const baselineFor = (repo, hotfix, choice) => {
  if (!hotfix) return REPOS[repo].base;
  return choice === 'production' ? REPOS[repo].prod : choice === 'uat' ? 'uat' : REPOS[repo].base;
};
function activationPlan(t, choice) {
  const errors = []; const rows = [];
  const plans = Object.fromEntries(repoPlan(t).map((p) => [p.repo, p]));
  for (const repo of Object.keys(REPOS)) {
    const p = plans[repo];
    if (p && p.ambiguous) errors.push(repo + ': ' + p.cands.length + ' branches match ' + t.key + '. Choose one on the Overview tab.');
    if (p && p.chosen) rows.push({ repo, role: 'ticket', branch: p.chosen });
    else rows.push({ repo, role: 'baseline', branch: baselineFor(repo, isHotfix(t), choice) });
  }
  const overlayRow = rows.find((r) => REPOS[r.repo].overlayConsumer && rows.find((x) => x.repo === REPOS[r.repo].consumes && x.role === 'ticket'));
  const overlay = overlayRow ? { repo: overlayRow.repo, provider: REPOS[overlayRow.repo].consumes } : null;
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
  t.local.status = 'active'; t.prep = null;
  audit('activation.complete', key);
  return { ok: true, rows: plan.rows, records, overlay: plan.overlay, warnings };
}
function deactivate(key, target, by) {
  const t = tk(key);
  if (!t || !t.act) return { ok: false, errors: ['Nothing to restore for ' + key + '.'] };
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
  return { ok: true, restored, overlayReverted };
}

/* ---------------------------- integration ---------------------------- */
function prepare(key) {
  const t = tk(key);
  if (!t || status(t) !== 'active') return { ok: false, error: 'The ticket must be active (tested locally) before it can be integrated.' };
  const rows = touchedRepos(t).map((p) => {
    const repo = p.repo;
    const done = t.merges.find((m) => m.repo === repo);
    if (done) return { repo, outcome: 'alreadyPushed', commit: done.commit };
    if (S.sim.conflict === repo) return { repo, outcome: 'conflict', files: ['src/session.rs'], with: 'PROJ-127' };
    if (S.sim.leak === repo) return { repo, outcome: 'blocked', reason: 'overlay leak: composer.json adds a path repository (commit ' + hex('leak' + repo) + ')' };
    const files = t.prs.filter((x) => x.repo === repo).flatMap((x) => x.files);
    const overlaps = [];
    for (const o of S.tickets) {
      if (o.key === t.key || !o.merges.some((m) => m.repo === repo)) continue;
      for (const f of o.prs.filter((x) => x.repo === repo).flatMap((x) => x.files)) if (files.some((g) => g.path === f.path)) overlaps.push({ key: o.key, file: f.path });
    }
    return { repo, outcome: 'ready', branch: p.chosen, commits: 2 + files.length, files: files.length, uatBefore: hex('u' + repo + S.seq), merge: hex('m' + repo + key + S.seq), overlaps };
  });
  t.prep = { at: S.ms, rows };
  audit('integration.prepare', key, null, rows.every((r) => r.outcome === 'ready' || r.outcome === 'alreadyPushed') ? 'success' : 'failure', rows.map((r) => r.repo + ':' + r.outcome).join(' '));
  return { ok: true, prep: t.prep };
}
const prepPushable = (t) => !!t.prep && t.prep.rows.some((r) => r.outcome === 'ready') && t.prep.rows.every((r) => r.outcome === 'ready' || r.outcome === 'alreadyPushed');
function push(key) {
  const t = tk(key); const prep = t && t.prep;
  if (!prep || !prepPushable(t)) return { ok: false, error: 'Nothing is ready to push. Prepare the integration again.' };
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
  return { ok: true, results, finalized };
}
function rerun(key, repo) {
  const t = tk(key); const d = t && t.deploy[repo]; if (!d) return { ok: false };
  d.run = ++S.run; d.state = 'pending'; d.since = S.ms; d.step = 'Queued'; if (S.sim.failDeploy === repo) S.sim.failDeploy = null;
  audit('bitbucket.pipeline_rerun', key, repo, 'success', 'new run #' + d.run); return { ok: true };
}
function stepDeploy() {
  for (const t of S.tickets) for (const [repo, d] of Object.entries(t.deploy)) {
    if (d.state === 'deployed' || d.state === 'failed') continue;
    const el = S.ms - d.since;
    if (el < 3000) { d.state = 'pending'; d.step = 'Queued'; }
    else if (el < 8000) { d.state = 'running'; d.step = el < 5000 ? 'Build' : el < 6500 ? 'Test' : 'Deploy alpha'; }
    else {
      d.state = S.sim.failDeploy === repo ? 'failed' : 'deployed'; d.step = d.state === 'failed' ? 'Deploy alpha' : 'Deploy alpha';
      toast(t.key + ' ' + repo + ' pipeline #' + d.run + (d.state === 'failed' ? ' failed' : ' deployed to alpha'), d.state === 'failed' ? 'bad' : 'ok');
    }
  }
}
function advance(ms) { S.ms += ms; stepDeploy(); S.ui.toasts = S.ui.toasts.filter((x) => x.until > S.ms); }

const allDeployed = (t) => t.merges.length > 0 && touchedRepos(t).every((p) => t.merges.some((m) => m.repo === p.repo)) && t.merges.every((m) => t.deploy[m.repo] && t.deploy[m.repo].state === 'deployed');
const anyFailed = (t) => t.merges.filter((m) => t.deploy[m.repo] && t.deploy[m.repo].state === 'failed');
const anyRunning = (t) => t.merges.some((m) => t.deploy[m.repo] && ['pending', 'running'].includes(t.deploy[m.repo].state));

/* ---------------------------- announce ---------------------------- */
const draftOf = (t, kind) => t.drafts.filter((d) => d.kind === kind).slice(-1)[0] || null;
function composeDraft(key) {
  const t = tk(key); if (!t || !allDeployed(t)) return { ok: false, error: 'Every touched repo must be deployed first.' };
  const lines = t.merges.map((m) => { const pr = t.prs.find((p) => p.repo === m.repo); const d = t.deploy[m.repo]; return '• ' + m.repo + '  PR #' + (pr ? pr.id : '-') + ' · pipeline #' + d.run + ' · https://ci.example/' + REPOS[m.repo].host + '/' + d.run; });
  t.drafts.push({ id: 'd' + nextSeq(), kind: 'comment', body: 'Deployed to alpha:\n' + lines.join('\n'), status: 'draft' });
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
function postFreeComment(key, text) {
  const t = tk(key); t.comments.push({ n: maxN(t) + 1, who: ME, at: 'today ' + clock(), body: text.split('\n').map((x) => P(x)) });
  t.seenN = maxN(t); audit('jira.comment', key, null, 'success', 'free comment'); return { ok: true };
}

/* ---------------------------- review writes ---------------------------- */
function approve(key, repo, prId) {
  const t = tk(key); const pr = t.prs.find((p) => p.repo === repo && p.id === prId); const me = pr.reviewers.find((r) => r.name === ME);
  me.approved = true; audit('bitbucket.pr_approve', key, repo, 'success', '#' + prId);
  if (t.prs.every((p) => p.reviewers.find((r) => r.name === ME).approved)) { t.local.status = 'done'; audit('ticket.done', key); }
  return { ok: true };
}
function requestChanges(key, repo, prId, text) {
  const t = tk(key); const pr = t.prs.find((p) => p.repo === repo && p.id === prId);
  pr.threads.push({ id: 'th' + nextSeq(), file: pr.files[0].path, line: null, author: ME, text: 'Changes requested: ' + text, resolved: false, replies: [] });
  audit('bitbucket.pr_request_changes', key, repo, 'success', '#' + prId); return { ok: true };
}
function addThread(key, prId, file, line, text) {
  const t = tk(key); const pr = t.prs.find((p) => p.id === prId);
  pr.threads.push({ id: 'th' + nextSeq(), file, line, author: ME, text, resolved: false, replies: [] });
  audit('bitbucket.pr_comment', key, pr.repo, 'success', file + ':' + line); return { ok: true };
}

/* ---------------------------- fake sync ---------------------------- */
function syncStart() {
  if (S.sync.running) return { ok: false };
  S.sync.running = true; S.sync.lastAttempt = S.ms; return { ok: true };
}
function syncFinish() {
  const report = []; S.sync.running = false;
  if (S.sim.offline) { S.sync.lastError = 'network unreachable'; S.sync.report = [{ src: 'Jira (acli)', ok: false, text: 'network unreachable, cache kept' }, { src: 'Bitbucket (bkt)', ok: false, text: 'network unreachable, cache kept' }]; audit('sync', null, null, 'failure', 'offline'); return { ok: false, report: S.sync.report }; }
  report.push({ src: 'Jira (acli)', ok: true, text: 'pool refreshed' });
  if (S.providers.bkt.ready) report.push({ src: 'Bitbucket (bkt)', ok: true, text: 'PRs and pipelines refreshed' });
  else report.push({ src: 'Bitbucket (bkt)', ok: false, text: 'signed out, cache kept. Run bkt auth login' });
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
  newCommits(key) { const t = tk(key); t.prs.forEach((p) => { p.updatedSeq += 1; }); t.newCommits = true; toast('New commits pushed to the branches of ' + key, 'info'); },
  toggle(name, val) { S.sim[name] = val; },
  bkt(ready) { S.providers.bkt.ready = ready; },
  jira(ready) { S.providers.jira.ready = ready; },
};

/* ============================ next-action rules ============================
   Simplified from the real engine: pure over S, ranked, each with a stated reason. */
const BANDS = { deploy_failed: 1000, integration_blocked: 990, returned_to_review: 850, returned_mention: 840, re_review: 830, start_review: 820, finish_review: 810,
  approve_prs: 690, integrate_ready: 680, push_ready: 675, compose_deploy_comment: 670, post_deploy_comment: 665, transition_alpha: 660, remerge_needed: 645,
  activate_reviewed: 640, park_active: 620, deploy_waiting: 600, claim_new: 480, adapter_unavailable: 299, sync_stale: 1200 };

function suggest() {
  const out = []; const act = activeTicket();
  const push_ = (s) => { out.push(Object.assign({ level: 'local', info: false }, s, { priority: (BANDS[s.rule] || 100) + (s.rank ? -s.rank * 10 : 0) })); };

  // sync
  const age = minsAgo(S.sync.lastOk); const sinceAttempt = minsAgo(S.sync.lastAttempt);
  if (!S.sync.running && ((S.sync.lastError && sinceAttempt >= 15) || (!S.sync.lastError && age >= 6 && sinceAttempt >= 1)))
    push_({ id: '-:sync_stale:all', ticket: null, rule: 'sync_stale', title: 'Sync now', reason: 'Data was last refreshed ' + age + ' min ago.', level: 'auto', act: { do: 'sync' }, hash: 'sync' });

  for (const t of S.tickets) {
    const st = status(t); const hot = isHotfix(t); const key = t.key;
    const need = (rule, sug) => push_(Object.assign({ ticket: key, rule, hotfix: hot }, sug));

    // returned + mentioned, or any unseen mention
    const ms = unseenMentions(t);
    if (ms.length && ['integrated', 'done', null, 'claimed', 'reviewing', 'parked', 'active'].includes(st))
      need('returned_mention', { id: key + ':returned_mention', title: (t.jira === JIRA.returned ? 'Returned: you were mentioned' : 'You were mentioned'), reason: ms[ms.length - 1].who + ' mentioned you on ' + key + ' (' + ms.length + ' unseen comment' + (ms.length > 1 ? 's' : '') + ').', info: true, act: { do: 'open', key, tab: 'overview' }, hash: 'm' + ms.length });
    if (t.jira === JIRA.review && (st === 'integrated' || st === 'done') && !t.newCommits && draftOf(t, 'comment') && draftOf(t, 'comment').status === 'posted' && t.merges.length)
      need('returned_to_review', { id: key + ':returned_to_review', title: 'Back in the Review column', reason: key + ' returned to Review after alpha. Treat it as new: claim it again.', act: { do: 'reclaim', key }, hash: 'rr' });

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
      if (t.prep && prepPushable(t)) need('push_ready', { id: key + ':push_ready', title: 'Review and push to uat', reason: 'Merge ready in ' + t.prep.rows.filter((r) => r.outcome === 'ready').map((r) => r.repo).join(', ') + '. Pushing deploys to alpha.', level: 'external', act: { do: 'pushAsk', key }, hash: 'p' + t.prep.at });
      if (prepped) t.prep.rows.filter((r) => r.outcome === 'conflict' || r.outcome === 'blocked').forEach((r) => need('integration_blocked', { id: key + ':blocked:' + r.repo, title: r.outcome === 'conflict' ? 'Resolve the uat conflict in ' + r.repo : 'Integration blocked in ' + r.repo, reason: r.outcome === 'conflict' ? 'Merging into uat conflicts with ' + r.with + ' in ' + r.files.join(', ') + '. Nothing was pushed.' : r.reason, info: true, act: { do: 'open', key, tab: 'ship' }, hash: r.outcome + r.repo }));
    }

    // A returned ticket is only your business if you are mentioned (handled above) or it fails to deploy.
    const sentBack = t.jira === JIRA.returned;
    if (t.merges.length) {
      for (const m of anyFailed(t)) {
        const d = t.deploy[m.repo];
        need('deploy_failed', { id: key + ':deploy_failed:' + m.repo, title: 'Deploy failed in ' + m.repo, reason: 'The deploy step of pipeline #' + d.run + ' failed after your push to uat.', info: false, level: 'external', needs: 'bkt', act: { do: 'rerunAsk', key, repo: m.repo }, hash: 'df' + d.run });
      }
      if (sentBack) continue;
      if (anyRunning(t) && !anyFailed(t).length) need('deploy_waiting', { id: key + ':deploy_waiting', title: 'Waiting for pipelines', reason: t.merges.filter((m) => ['pending', 'running'].includes(t.deploy[m.repo].state)).map((m) => m.repo + ' #' + t.deploy[m.repo].run + ' (' + t.deploy[m.repo].step + ')').join(', ') + '.', info: true, level: 'auto', act: { do: 'open', key, tab: 'ship' }, hash: 'w' });
      const d = draftOf(t, 'comment');
      if (allDeployed(t) && !d) need('compose_deploy_comment', { id: key + ':compose', title: 'Draft the deploy comment', reason: 'Every touched repo is deployed to alpha. The draft lists each repo with its PR and pipeline.', act: { do: 'composeDraft', key }, hash: 'c' });
      if (d && d.status === 'draft') need('post_deploy_comment', { id: key + ':post', title: 'Post the deploy comment to Jira', reason: 'A draft is ready. Edit it, then post.', level: 'external', needs: 'acli', act: { do: 'postAsk', key, draft: d.id }, hash: 'pc' + d.id });
      if (d && d.status === 'posted' && t.jira === JIRA.review) need('transition_alpha', { id: key + ':transition', title: 'Move to ' + JIRA.alpha, reason: 'The comment is posted. Transition ' + JIRA.review + ' → ' + JIRA.alpha + '.', level: 'external', needs: 'acli', act: { do: 'transitionAsk', key }, hash: 't' });
    }
    if (st === 'integrated' && t.newCommits && !act) need('remerge_needed', { id: key + ':remerge', title: hot ? 'Re-merge needed (choose baseline)' : 'Re-merge needed', reason: 'The ticket branches have commits that uat lacks after your recorded merge. Activate it again, test, and integrate again.', act: { do: 'activate', key }, hash: 'rm' });
    if (st === 'integrated' && isSignedOff(t)) {
      t.prs.filter((p) => !p.reviewers.find((r) => r.name === ME).approved).forEach((p) => need('approve_prs', { id: key + ':approve:' + p.repo + '#' + p.id, title: 'Approve ' + REPOS[p.repo].host + ' #' + p.id, reason: key + ' passed testing (Jira: ' + t.jira + '). Approval is the last gate.', level: 'external', needs: 'bkt', act: { do: 'approveAsk', key, repo: p.repo, pr: p.id }, hash: 'ap' + p.id }));
    }
  }

  // claim queue
  const pool = S.tickets.filter((t) => !t.local && t.jira === JIRA.review).sort((a, b) => (PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority]) || a.key.localeCompare(b.key, undefined, { numeric: true }));
  pool.forEach((t, i) => push_({ id: t.key + ':claim_new', ticket: t.key, rule: 'claim_new', hotfix: isHotfix(t), rank: i, title: 'Claim ' + t.key, reason: 'In the Review column, priority ' + t.priority + (isHotfix(t) ? ', targets production (hotfix)' : '') + ', #' + (i + 1) + ' in your queue.', act: { do: 'claim', key: t.key }, hash: 'q', facts: { queue: i } }));

  // hotfix boost, adapter availability
  const res = out.map((s) => {
    let p = s.priority; if (s.hotfix && s.priority >= 400 && s.rule !== 'park_active') p += 200;
    if (s.needs && !S.providers[s.needs === 'acli' ? 'jira' : 'bkt'].ready) {
      const name = s.needs === 'acli' ? 'acli' : 'bkt';
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
const visibleSuggestions = () => suggest().filter((s) => s.state === 'open' || s.state === 'resurfaced');
function dismiss(id) { const s = suggest().find((x) => x.id === id); S.responses[id] = { kind: 'dismissed', hash: s ? s.hash : '' }; audit('suggestion.dismiss', s && s.ticket, null, 'success', id); }
function snooze(id, mins) { S.responses[id] = { kind: 'snoozed', until: S.ms + mins * MS_PER_MIN }; audit('suggestion.snooze', null, null, 'success', id + ' ' + mins + 'm'); }

/* Test hook (node only) */
if (typeof window === 'undefined') {
  globalThis.__DE = { get S() { return S; }, reclaim, syncStart, syncFinish, reset() { S = newState(); script.length = 0; syncScript().forEach((e) => script.push(e)); }, tk, claim, startReview, markReviewed, toggleChecklist, activate, deactivate, prepare, push, rerun, advance, allDeployed,
    composeDraft, postComment, transition, approve, syncNow, SIM, suggest, visibleSuggestions, dismiss, snooze, setLink, markSeen, repoPlan, isHotfix, draftOf, postFreeComment, addThread, requestChanges, activeTicket, unseenMentions };
}
