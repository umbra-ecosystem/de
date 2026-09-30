'use strict';
/* ============================ view helpers and shell ============================ */
const esc = (s) => String(s == null ? '' : s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
const attr = (o) => Object.entries(o || {}).map(([k, v]) => ' data-' + k + '="' + esc(v) + '"').join('');
const btn = (label, act, args, cls, extra) => '<button class="btn ' + (cls || '') + '" data-act="' + act + '"' + attr(args) + (extra && extra.disabled ? ' disabled' : '') + (extra && extra.title ? ' title="' + esc(extra.title) + '"' : '') + '>' + label + '</button>';
const pill = (text, cls) => '<span class="pill ' + (cls || '') + '">' + esc(text) + '</span>';
const link = (label, act, args, cls) => '<a href="#" class="lnk ' + (cls || '') + '" data-act="' + act + '"' + attr(args) + '>' + label + '</a>';
const jiraCls = (s) => (s === JIRA.returned ? 'warn' : s === JIRA.alpha ? 'acc' : JIRA.signed.includes(s) ? 'ok' : '');
const localCls = (s) => ({ active: 'acc', integrated: 'ok', parked: '', done: 'ok', reviewing: 'warn', claimed: '' }[s] || '');
const LOCAL_LABEL = { claimed: 'Claimed', reviewing: 'Reviewing', active: 'Active', parked: 'Parked', integrated: 'Integrated', done: 'Done' };
const prioCls = (p) => (p === 'Highest' ? 'bad' : p === 'High' ? 'warn' : '');

function blocksHtml(blocks) {
  const inline = (x) => esc(x).replace(/@\[([^\]]+)\]/g, (m, n) => '<span class="mention' + (n.toLowerCase() === 'you' ? ' me' : '') + '">@' + (n.toLowerCase() === 'you' ? 'you' : n) + '</span>').replace(/(https?:\/\/[^\s<]+)/g, '<span class="url">$1</span>');
  return blocks.map((b) => {
    if (b.t === 'p') return '<p>' + inline(b.x) + '</p>';
    if (b.t === 'h') return '<h4>' + esc(b.x) + '</h4>';
    if (b.t === 'ul') return '<ul>' + b.items.map((i) => '<li>' + inline(i) + '</li>').join('') + '</ul>';
    if (b.t === 'code') return '<pre class="code">' + esc(b.x) + '</pre>';
    return '';
  }).join('');
}

/* ---------------- ticket progress (idea: one stepper on every ticket screen) ---------------- */
const STEPS = ['Claimed', 'Reviewed', 'Testing', 'On uat', 'Deployed', 'Announced', 'Alpha', 'Signed off', 'Approved'];
function progressIndex(t) {
  const st = status(t); const d = draftOf(t, 'comment');
  if (st === 'done') return 9;
  if (st === 'integrated' || (t.merges.length && st !== 'active')) {
    if (isSignedOff(t)) return 7;
    if (t.jira === JIRA.alpha) return 6;
    if (d && d.status === 'posted') return 5;
    if (allDeployed(t)) return 4;
    return 3;
  }
  if (st === 'active') return 2;
  if (t.reviewed) return 2 - 0.5;
  if (st) return 1;
  return 0;
}
function stepper(t) {
  const idx = progressIndex(t);
  return '<ol class="stepper">' + STEPS.map((s, i) => '<li class="' + (i < idx ? 'done' : i === Math.floor(idx) || (idx % 1 && i === Math.ceil(idx)) ? 'now' : '') + '"><i></i><span>' + s + '</span></li>').join('') + '</ol>';
}

/* ---------------- left sidebar (fixed) ---------------- */
const groupOf = {
  pool: (t) => !t.local && t.jira === JIRA.review,
  mine: (t) => ['claimed', 'reviewing'].includes(status(t)) && t.jira !== JIRA.returned,
  active: (t) => status(t) === 'active',
  parked: (t) => status(t) === 'parked',
  awaiting: (t) => status(t) === 'integrated' && t.jira !== JIRA.returned,
  returned: (t) => t.jira === JIRA.returned,
  done: (t) => status(t) === 'done',
  all: () => true,
};
const GROUP_LABEL = { pool: 'Review pool', mine: 'In review', active: 'Active', parked: 'Parked', awaiting: 'Awaiting alpha', returned: 'Returned', done: 'Done', all: 'All tickets' };
const count = (g) => S.tickets.filter(groupOf[g]).length;

function leftNav() {
  const r = S.ui.route; const sug = visibleSuggestions().filter((s) => !s.info || s.level !== 'auto').length;
  const it = (label, route, n, extra) => '<a href="#" class="nav ' + ((r.name === route.name && (route.group ? r.group === route.group : true) && (route.name !== 'ticket')) ? 'on' : '') + '" data-act="go"' + attr({ name: route.name, group: route.group || '' }) + '><span>' + label + '</span>' + (n != null ? '<b>' + n + '</b>' : '') + (extra || '') + '</a>';
  const health = (name, ok) => '<div class="hp"><i class="dot ' + (ok ? 'ok' : 'warn') + '"></i><span>' + name + '</span><em>' + (ok ? 'ready' : 'signed out') + '</em></div>';
  const act = activeTicket();
  return '<div class="brand"><span class="logo">de</span><span class="proto">prototype</span></div>' +
    it('Next', { name: 'next' }, sug) +
    '<div class="grp">Tickets</div>' + ['pool', 'mine', 'active', 'parked', 'awaiting', 'returned', 'done'].map((g) => it(GROUP_LABEL[g], { name: 'tickets', group: g }, count(g))).join('') +
    '<div class="grp">Environment</div>' + it('On uat', { name: 'env' }, null) + it('Workspace', { name: 'workspace' }, null) +
    '<div class="grp">System</div>' + it('Audit log', { name: 'audit' }, S.audit.length) + it('Settings', { name: 'settings' }, null) + it('Change ideas', { name: 'ideas' }, null) +
    '<div class="spacer"></div>' +
    '<div class="prov">' + health('acli · Jira', S.providers.jira.ready) + health('bkt · Bitbucket', S.providers.bkt.ready) +
    '<div class="fresh">' + (S.sync.running ? 'Syncing…' : S.sync.lastError ? '<span class="warn-t">Last sync: ' + esc(S.sync.lastError) + '</span>' : 'Synced ' + minsAgo(S.sync.lastOk) + ' min ago') + '</div></div>';
}

/* ---------------- top bar and now bar ---------------- */
function crumb() {
  const r = S.ui.route;
  if (r.name === 'ticket') { const t = tk(r.key); return '<a href="#" data-act="go" data-name="tickets" data-group="all">Tickets</a><i>/</i><b class="key">' + esc(r.key) + '</b><span class="ct">' + esc(t ? t.title : '') + '</span>'; }
  const L = { next: 'Next', tickets: GROUP_LABEL[r.group] || 'Tickets', env: 'On uat', workspace: 'Workspace', audit: 'Audit log', settings: 'Settings', ideas: 'Change ideas' };
  return '<b>' + esc(L[r.name] || '') + '</b>';
}
function topBar() {
  return '<div class="top"><button class="icon mob" data-act="toggleLeft" aria-label="Menu">☰</button><div class="crumb">' + crumb() + '</div><span class="sp"></span>' +
    '<button class="search" data-act="palette">⌕ <span>Search or jump</span> <kbd>⌘K</kbd></button>' +
    '<button class="btn" data-act="sync"' + (S.sync.running ? ' disabled' : '') + '>' + (S.sync.running ? '<span class="spin"></span> Syncing' : '↻ Sync') + '</button>' +
    '<button class="btn ' + (S.ui.dev ? 'p' : '') + '" data-act="toggleDev">Simulate</button>' +
    '<button class="icon" data-act="toggleRight" aria-label="Toggle details" title="Toggle right sidebar">▤</button></div>';
}
function nowBar() {
  const t = activeTicket(); if (!t) return '';
  const ov = t.act && t.act.overlay;
  return '<div class="now"><span class="pill acc">▶ ACTIVE</span><a href="#" class="key" data-act="go" data-name="ticket" data-key="' + t.key + '" data-tab="test">' + t.key + '</a><span class="nt">' + esc(t.title) + '</span><span class="mono mute">' + fmtDur(durationMs(t)) + '</span>' +
    (ov ? '<span class="pill warn" title="composer path repository in ' + ov.repo + '. Must be reverted before anything is pushed.">⚡ overlay on ' + ov.repo + '</span>' : '') +
    '<span class="sp"></span>' + btn('Open', 'go', { name: 'ticket', key: t.key, tab: 'test' }, 'sm') + btn('Park…', 'parkAsk', { key: t.key }, 'sm') + '</div>';
}

/* ---------------- right sidebar (contextual) ---------------- */
const kv = (k, v) => '<div class="kv"><span>' + k + '</span><b>' + v + '</b></div>';
function activityFor(key, n) {
  const rows = S.audit.filter((a) => !key || a.ticket === key).slice(0, n || 6);
  if (!rows.length) return '<div class="faint pad">No activity yet.</div>';
  return '<ul class="act">' + rows.map((a) => '<li><span class="mono faint">' + a.at + '</span> <span>' + esc(a.action) + '</span>' + (a.repo ? ' <em>' + esc(a.repo) + '</em>' : '') + (a.outcome === 'failure' ? ' ' + pill('failed', 'bad') : '') + '</li>').join('') + '</ul>';
}
function syncCard() {
  const rep = S.sync.report.length ? '<ul class="rep">' + S.sync.report.map((r) => '<li>' + (r.ok ? '<i class="dot ok"></i>' : '<i class="dot bad"></i>') + '<span><b>' + esc(r.src) + '</b> ' + esc(r.text) + '</span></li>').join('') + '</ul>' : '<div class="faint">No sync yet this session.</div>';
  return '<section class="rc"><h5>Sync</h5><div class="row sb"><span>' + (S.sync.running ? 'Syncing…' : 'Synced ' + minsAgo(S.sync.lastOk) + ' min ago') + '</span>' + btn('↻ Sync now', 'sync', {}, 'sm', { disabled: S.sync.running }) + '</div>' + rep + '<div class="faint small">Fake sync: each run applies the next scripted arrival (a new ticket, a new mention). Sync is read-only and never deletes cache on failure.</div></section>';
}
function rightNext() {
  const t = activeTicket();
  const now = t ? '<section class="rc"><h5>Now</h5><div><a href="#" class="key" data-act="go" data-name="ticket" data-key="' + t.key + '" data-tab="test">' + t.key + '</a> ' + esc(t.title) + '</div><div class="mute small">' + fmtDur(durationMs(t)) + ' · ' + t.checklist.filter((c) => c.done).length + '/' + t.checklist.length + ' checked</div><div class="row">' + btn('Open', 'go', { name: 'ticket', key: t.key, tab: 'test' }, 'sm') + '</div></section>'
    : '<section class="rc"><h5>Now</h5><div class="mute">No active ticket. Activate a reviewed ticket to test it locally.</div></section>';
  const q = S.tickets.filter(groupOf.pool).sort((a, b) => PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority]).slice(0, 4);
  const queue = '<section class="rc"><h5>Claim queue</h5>' + (q.length ? q.map((x) => '<div class="qi"><a href="#" class="key" data-act="go" data-name="ticket" data-key="' + x.key + '" data-tab="overview">' + x.key + '</a> ' + pill(x.priority, prioCls(x.priority)) + (isHotfix(x) ? ' ' + pill('HOTFIX', 'hot') : '') + '<div class="mute small">' + esc(x.title) + '</div></div>').join('') : '<div class="faint">Empty.</div>') + '</section>';
  return now + syncCard() + queue + '<section class="rc"><h5>Recent activity</h5>' + activityFor(null, 6) + '</section>';
}
function rightTicket(t) {
  const hot = isHotfix(t); const st = status(t);
  const prs = t.prs.map((p) => { const d = t.deploy[p.repo]; const ap = p.reviewers.filter((r) => r.approved).length;
    return '<div class="qi"><div class="row sb"><span class="mono">' + esc(p.repo) + ' #' + p.id + '</span>' + pill('open', '') + '</div><div class="mute small">' + esc(p.src) + ' → ' + pill(p.dst, p.dst === REPOS[p.repo].prod && REPOS[p.repo].prod !== REPOS[p.repo].base ? 'hot' : '') + '</div><div class="small">' + ap + '/' + p.reviewers.length + ' approvals' + (d ? ' · pipeline #' + d.run + ' ' + pill(d.state, d.state === 'deployed' ? 'ok' : d.state === 'failed' ? 'bad' : 'warn') : '') + '</div></div>'; }).join('');
  return '<section class="rc"><h5>Status</h5>' + kv('Jira', pill(t.jira, jiraCls(t.jira))) + kv('Local', st ? pill(LOCAL_LABEL[st], localCls(st)) : pill('not claimed', '')) + kv('Kind', hot ? pill('HOTFIX', 'hot') : 'Normal') + kv('Time', fmtDur(durationMs(t))) + kv('Reviewed', t.reviewed ? pill('yes', 'ok') : 'no') + '</section>' +
    '<section class="rc"><h5>Details</h5>' + kv('Type', esc(t.type)) + kv('Priority', pill(t.priority, prioCls(t.priority))) + kv('Assignee', esc(t.assignee)) + kv('Reporter', esc(t.reporter)) + kv('Sprint', esc(t.sprint)) + kv('Epic', esc(t.epic)) + kv('Fix version', esc(t.fixVersion)) + kv('Estimate', esc(t.estimate)) +
    kv('Components', t.components.map((c) => pill(c)).join(' ')) + kv('Labels', t.labels.map((c) => pill(c)).join(' ') || '–') + kv('Created', esc(t.created)) + kv('Updated', esc(t.updated)) + '</section>' +
    '<section class="rc"><h5>Pull requests</h5>' + prs + '</section>' +
    (t.links.length ? '<section class="rc"><h5>Linked issues</h5>' + t.links.map((l) => '<div class="qi"><span class="faint small">' + esc(l.rel) + '</span> ' + (tk(l.key) ? link('<span class="key">' + esc(l.key) + '</span>', 'go', { name: 'ticket', key: l.key, tab: 'overview' }) : '<span class="key">' + esc(l.key) + '</span>') + ' ' + pill(l.status, jiraCls(l.status)) + '<div class="mute small">' + esc(l.title) + '</div></div>').join('') + '</section>' : '') +
    '<section class="rc"><h5>Activity</h5>' + activityFor(t.key, 8) + '</section>';
}
function rightPlain(name) {
  const hints = {
    tickets: 'Ordered by Jira priority, then your own order. Open a ticket to see its description, comments, PRs and pipelines.',
    env: 'Shows what is on uat in each repo right now, and which tickets could collide. uat accumulates tickets, so this is the picture to check before pushing.',
    workspace: 'Start and stop keep the CLI meaning: dependency order, with a guard for uncommitted or unpushed work.',
    audit: 'Every external write is audited before it happens (an "attempted" entry) and after (the outcome). Append-only.',
    settings: 'The app never stores tokens. Each tool keeps its own login.',
    ideas: 'Proposals for changing the design. Ones marked built are already in this prototype.',
  };
  return '<section class="rc"><h5>About this screen</h5><div class="mute">' + hints[name] + '</div></section>' + syncCard() + '<section class="rc"><h5>Recent activity</h5>' + activityFor(null, 6) + '</section>';
}
function rightPanel() {
  const r = S.ui.route;
  if (r.name === 'ticket' && tk(r.key)) return rightTicket(tk(r.key));
  if (r.name === 'next') return rightNext();
  return rightPlain(r.name);
}
