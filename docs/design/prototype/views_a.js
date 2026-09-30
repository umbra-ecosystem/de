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

/* ---------------- left panel ---------------- */
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
  const it = (label, route, n) => '<a href="#" class="nav ' + ((r.name === route.name && (route.group ? r.group === route.group : true) && (route.name !== 'ticket')) ? 'on' : '') + '" data-act="go"' + attr({ name: route.name, group: route.group || '' }) + '><span>' + label + '</span>' + (n ? '<b>' + n + '</b>' : '') + '</a>';
  return '<div class="ph">Tickets</div>' + it('Next', { name: 'next' }, sug) +
    ['pool', 'mine', 'active', 'parked', 'awaiting', 'returned', 'done'].map((g) => it(GROUP_LABEL[g], { name: 'tickets', group: g }, count(g))).join('') +
    '<div class="ph">Environment</div>' + it('On uat', { name: 'env' }) + it('Workspace', { name: 'workspace' }) +
    '<div class="ph">System</div>' + it('Audit log', { name: 'audit' }) + it('Settings', { name: 'settings' });
}

/* ---------------- title bar, tab bar, status bar ---------------- */
function crumb() {
  const r = S.ui.route;
  if (r.name === 'ticket') { const t = tk(r.key); return '<span class="key">' + esc(r.key) + '</span><span class="ct">' + esc(t ? t.title : '') + '</span>'; }
  const L = { next: 'Next', tickets: GROUP_LABEL[r.group] || 'Tickets', env: 'On uat', workspace: 'Workspace', audit: 'Audit log', settings: 'Settings' };
  return '<span>' + esc(L[r.name] || '') + '</span>';
}
function titleBar() {
  return '<div class="tb"><div class="lights"><i></i><i></i><i></i></div><button class="icon mob" data-act="toggleLeft" aria-label="Menu">☰</button><span class="tt-name">de</span><span class="sp"></span>' +
    '<button class="search" data-act="palette"><span>Go to…</span><kbd>⌘K</kbd></button><span class="sp"></span>' +
    '<button class="icon attnb' + (S.ui.attn ? ' on' : '') + '" data-act="toggleAttn">Needs attention' + (attention().length ? ' <b class="bdg">' + attention().length + '</b>' : '') + '</button>' +
    '<button class="icon' + (S.ui.dev ? ' on' : '') + '" data-act="toggleDev" title="Simulate">Simulate</button>' +
    '<button class="icon" data-act="toggleRight" aria-label="Toggle details" title="Toggle right panel">▥</button></div>';
}
/* ---------------- open tickets as tabs (preview tab is replaced until pinned) ---------------- */
function syncTabs() {
  const r = S.ui.route; S.ui.tabs = S.ui.tabs || [];
  if (r.name !== 'ticket') { S.ui.screen = r; return; }
  let t = S.ui.tabs.find((x) => x.key === r.key);
  if (!t) {
    t = { key: r.key, pinned: false, tab: r.tab || 'overview' };
    const pv = S.ui.tabs.findIndex((x) => !x.pinned);
    if (pv >= 0) S.ui.tabs[pv] = t; else S.ui.tabs.push(t);
  }
  t.tab = r.tab || 'overview';
}
function tabBar() {
  const r = S.ui.route; const scr = S.ui.screen || { name: 'next' };
  const L = { next: 'Next', tickets: GROUP_LABEL[scr.group] || 'Tickets', env: 'On uat', workspace: 'Workspace', audit: 'Audit log', settings: 'Settings' };
  const first = '<a href="#" class="tab ' + (r.name !== 'ticket' ? 'on' : '') + '" data-act="go" data-name="' + scr.name + '" data-group="' + (scr.group || '') + '">' + esc(L[scr.name] || '') + '</a>';
  const act = activeTicket();
  const tabs = (S.ui.tabs || []).map((x) => {
    const t = tk(x.key); if (!t) return '';
    const bad = anyFailed(t).length || uatConflicts(t).length || prWaitExpired(t) || thWaitExpired(t); const un = unseenCount(t) > 0 && !(r.name === 'ticket' && r.key === x.key);
    const mark = act && act.key === x.key ? '<span class="tplay">▶</span>' : bad ? '<i class="dot bad"></i>' : un ? '<i class="dot" style="background:var(--accent)"></i>' : '';
    return '<span class="tab ' + (r.name === 'ticket' && r.key === x.key ? 'on' : '') + (x.pinned ? '' : ' pv') + '" data-act="go" data-name="ticket" data-key="' + x.key + '" data-tab="' + x.tab + '" data-dbl="1" data-mid="1">' + mark + '<span class="key">' + x.key + '</span><span class="ct">' + esc(t.title.length > 18 ? t.title.slice(0, 17) + '…' : t.title) + '</span><button class="x" data-act="closeTab" data-key="' + x.key + '" aria-label="Close ' + x.key + '">×</button></span>';
  }).join('');
  return '<div class="tabstrip">' + first + tabs + '</div>';
}
function statusBar() {
  const t = activeTicket(); const ov = t && t.act && t.act.overlay;
  const health = (name, ok) => '<span class="sbi"><i class="dot ' + (ok ? 'ok' : 'warn') + '"></i>' + name + '</span>';
  const sync = S.sync.running ? '<span class="spin"></span> Syncing' : S.sync.lastError ? '<span class="warn-t">' + esc(S.sync.lastError) + '</span>' : 'Synced ' + minsAgo(S.sync.lastOk) + 'm ago';
  return '<div class="status"><span class="sbl">' + 
    (t ? '<a href="#" class="sbi hl" data-act="go" data-name="ticket" data-key="' + t.key + '" data-tab="test">▶ ' + t.key + ' <span class="mono">' + fmtDur(durationMs(t)) + '</span></a>' : '') +
    (ov ? '<span class="sbi warnb" title="Must be reverted before anything is pushed">overlay: ' + ov.repo + '</span>' : '') +
    (t ? '<a href="#" class="sbi" data-act="parkAsk" data-key="' + t.key + '">Park…</a>' : '') +
    '</span><span class="sp"></span><span class="sbl">' + Object.keys(REPOS).filter((r) => lockOf(r)).map((r) => '<span class="sbi ' + (lockOf(r).kind === 'stale' ? 'badb' : 'warnb') + '" title="' + esc(lockText(lockOf(r))) + '">◼ ' + r + ': ' + (lockOf(r).kind === 'stale' ? 'stale lock' : lockOf(r).kind === 'sync' ? 'fetching' : lockOf(r).kind === 'external' ? esc(lockOf(r).by) : esc(lockOf(r).op)) + '</span>').join('') + Object.keys(S.staleRepos).map((r) => '<span class="sbi warnb" title="' + esc(S.staleRepos[r].by) + '">' + r + ': not refreshed</span>').join('') + Object.keys(S.pending).map((r) => '<span class="sbi" title="' + esc(S.pending[r].by) + '">' + r + ': waiting to fetch</span>').join('') + health('Jira', S.providers.jira.ready) + health('GitHub', S.providers.gh.ready) +
    '<a href="#" class="sbi" data-act="sync"' + (S.sync.running ? ' disabled' : '') + '>' + sync + ' ↻</a></span></div>';
}

/* ---------------- right sidebar (contextual) ---------------- */
const kv = (k, v) => '<div class="kv"><span>' + k + '</span><b>' + v + '</b></div>';
function activityFor(key, n) {
  const rows = S.audit.filter((a) => !key || a.ticket === key).slice(0, n || 6);
  if (!rows.length) return '<div class="faint pad">No activity yet.</div>';
  return '<ul class="act">' + rows.map((a) => '<li><span class="mono faint">' + a.at + '</span> <span>' + esc(a.action) + '</span>' + (a.repo ? ' <em>' + esc(a.repo) + '</em>' : '') + (a.outcome === 'failure' ? ' ' + pill('failed', 'bad') : '') + '</li>').join('') + '</ul>';
}
function syncCard() {
  if (!S.sync.report.length) return '';
  return '<section class="rc"><h5>Last sync</h5><ul class="rep">' + S.sync.report.map((r) => '<li>' + (r.ok ? '<i class="dot ok"></i>' : '<i class="dot bad"></i>') + '<span><b>' + esc(r.src) + '</b> ' + esc(r.text) + '</span></li>').join('') + '</ul></section>';
}
function rightNext() {
  const t = activeTicket();
  const now = t ? '<section class="rc"><h5>Now</h5><div><a href="#" class="key" data-act="go" data-name="ticket" data-key="' + t.key + '" data-tab="test">' + t.key + '</a> ' + esc(t.title) + '</div><div class="mute small">' + fmtDur(durationMs(t)) + ' · ' + t.checklist.filter((c) => c.done).length + '/' + t.checklist.length + ' checked</div><div class="row">' + btn('Open', 'go', { name: 'ticket', key: t.key, tab: 'test' }, 'sm') + '</div></section>'
    : '';
  const q = S.tickets.filter(groupOf.pool).sort((a, b) => PRIORITY_RANK[a.priority] - PRIORITY_RANK[b.priority]).slice(0, 4);
  const queue = '<section class="rc"><h5>Claim queue</h5>' + (q.length ? q.map((x) => '<div class="qi"><a href="#" class="key" data-act="go" data-name="ticket" data-key="' + x.key + '" data-tab="overview">' + x.key + '</a> ' + pill(x.priority, prioCls(x.priority)) + (isHotfix(x) ? ' ' + pill('HOTFIX', 'hot') : '') + '<div class="mute small">' + esc(x.title) + '</div></div>').join('') : '<div class="faint">Empty.</div>') + '</section>';
  const today = S.tickets.filter((x) => durationMs(x) > 0); const total = today.reduce((n, x) => n + durationMs(x), 0);
  const time = today.length ? '<section class="rc"><h5>Today</h5><div class="kv"><span>Total</span><b>' + fmtDur(total) + '</b></div>' + today.map((x) => '<div class="kv"><span><a href="#" class="key" data-act="go" data-name="ticket" data-key="' + x.key + '" data-tab="overview">' + x.key + '</a></span><b>' + fmtDur(durationMs(x)) + '</b></div>').join('') + '</section>' : '';
  return now + time + syncCard() + queue + '<section class="rc"><h5>Recent activity</h5>' + activityFor(null, 6) + '</section>';
}
function rightTicket(t) {
  const hot = isHotfix(t); const st = status(t);
  const prs = t.prs.map((p) => { const d = t.deploy[p.repo]; const ap = p.reviewers.filter((r) => r.approved).length;
    return '<div class="qi"><div class="row sb"><span class="mono">' + esc(p.repo) + ' #' + p.id + '</span>' + pill('open', '') + '</div><div class="mute small">' + esc(p.src) + ' → ' + pill(p.dst, p.dst === REPOS[p.repo].prod && REPOS[p.repo].prod !== REPOS[p.repo].base ? 'hot' : '') + '</div><div class="small">' + ap + '/' + p.reviewers.length + ' approvals' + (d ? ' · run #' + d.run + ' ' + pill(d.state, d.state === 'deployed' ? 'ok' : d.state === 'failed' ? 'bad' : 'warn') : '') + '</div></div>'; }).join('');
  return '<section class="rc"><h5>Status</h5>' + kv('Jira', pill(t.jira, jiraCls(t.jira))) + kv('Local', st ? pill(LOCAL_LABEL[st], localCls(st)) : pill('not claimed', '')) + kv('Kind', hot ? pill('HOTFIX', 'hot') : 'Normal') + kv('Time', fmtDur(durationMs(t))) + kv('Reviewed', t.reviewed ? pill('yes', 'ok') : 'no') + '</section>' +
    '<section class="rc"><h5>Details</h5>' + kv('Type', esc(t.type)) + kv('Priority', pill(t.priority, prioCls(t.priority))) + kv('Assignee', esc(t.assignee)) + kv('Reporter', esc(t.reporter)) + kv('Sprint', esc(t.sprint)) + kv('Epic', esc(t.epic)) + kv('Fix version', esc(t.fixVersion)) + kv('Estimate', esc(t.estimate)) +
    kv('Components', t.components.map((c) => pill(c)).join(' ')) + kv('Labels', t.labels.map((c) => pill(c)).join(' ') || '–') + kv('Created', esc(t.created)) + kv('Updated', esc(t.updated)) + '</section>' +
    '<section class="rc"><h5>Pull requests</h5>' + (prs || '<div class="faint">' + (hasPrGap(t) ? esc(gapText(t)) : 'None.') + '</div>') + '</section>' +
    (t.links.length ? '<section class="rc"><h5>Linked issues</h5>' + t.links.map((l) => '<div class="qi"><span class="faint small">' + esc(l.rel) + '</span> ' + (tk(l.key) ? link('<span class="key">' + esc(l.key) + '</span>', 'go', { name: 'ticket', key: l.key, tab: 'overview' }) : '<span class="key">' + esc(l.key) + '</span>') + ' ' + pill(l.status, jiraCls(l.status)) + '<div class="mute small">' + esc(l.title) + '</div></div>').join('') + '</section>' : '') +
    '<section class="rc"><h5>Activity</h5>' + activityFor(t.key, 8) + '</section>';
}
function rightPlain() {
  return syncCard() + '<section class="rc"><h5>Activity</h5>' + activityFor(null, 8) + '</section>';
}
function rightPanel() {
  const r = S.ui.route;
  if (r.name === 'ticket' && tk(r.key)) return rightTicket(tk(r.key));
  if (r.name === 'next') return rightNext();
  return rightPlain();
}

function attnPanel() {
  if (!S.ui.attn) return '';
  const items = attention();
  const row = (s) => '<div class="ai"><div class="row sb"><span class="row">' + (s.hotfix ? pill('HOTFIX', 'hot') : '') + (s.ticket ? '<a href="#" class="key" data-act="go" data-name="ticket" data-key="' + s.ticket + '" data-tab="overview">' + s.ticket + '</a>' : '') + '<b>' + esc(s.title) + '</b></span>' + btn(ACT_LABEL[s.act.do] || 'Do', 'sug', { id: s.id }, 'sm' + (s.level === 'external' ? ' p' : '')) + '</div><div class="mute small">' + esc(s.reason) + '</div></div>';
  return '<div class="attn-scrim" data-act="toggleAttn"></div><aside class="attnp"><div class="sh-h"><span>Needs attention</span><span class="row">' + btn('All next actions', 'go', { name: 'next' }, 'g sm') + '</span></div>' + (items.length ? items.map(row).join('') : '<div class="ai faint">Nothing needs you right now.</div>') + '</aside>';
}
