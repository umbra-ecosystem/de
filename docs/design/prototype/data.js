'use strict';
/* ============================ fake data ============================
   Everything here is invented. Mirrors the shapes the real core serves:
   tickets (Jira mirror + local tracking), PRs with source/destination, GitHub Actions runs, audit log. */

const ME = 'You';
const JIRA = { review: 'In Review', alpha: 'Alpha Testing', returned: 'Returned', signed: ['Done'] };
const PRIORITY_RANK = { Highest: 0, High: 1, Medium: 2, Low: 3 };

/* Workspace repos. `base` is the fallback branch, `prod` the production branch (hotfix PRs target it).
   web consumes api-client through a composer path overlay while testing. */
const REPOS = {
  'api-client': { base: 'develop', prod: 'master', host: 'acme/api-client', services: 3, overlayConsumer: false, checks: ['Unit tests pass', 'Lint clean'] },
  web:          { base: 'develop', prod: 'main',   host: 'acme/web',        services: 2, overlayConsumer: true, consumes: 'api-client', checks: ['Unit tests pass', 'Build succeeds'] },
  worker:       { base: 'develop', prod: 'master', host: 'acme/worker',     services: 1, overlayConsumer: false, checks: ['Unit tests pass'] },
  docs:         { base: 'master',  prod: 'master', host: 'acme/docs',       services: 0, overlayConsumer: false, checks: [] },
};

const P  = (x) => ({ t: 'p', x });
const H  = (x) => ({ t: 'h', x });
const UL = (...items) => ({ t: 'ul', items });
const CODE = (x) => ({ t: 'code', x });

/* diff line: kind, old no, new no, text */
const L = (k, o, n, x) => ({ k, o, n, x });

function seedTickets() {
  const tickets = [];
  const add = (t) => { tickets.push(Object.assign({
    local: null, reviewed: false, reviewedSeq: null, checklist: [], notes: '', act: null, timeMs: 0,
    merges: [], deploy: {}, drafts: [], prep: null, pre: {}, link: {}, excl: {}, seenN: 0, viewedAt: null,
    newCommits: false, handledN: 0, cands: {}, prs: [], attachments: [], links: [], subtasks: [], ac: [],
    labels: [], components: [], comments: [],
  }, t)); };

  add({
    key: 'PROJ-142', type: 'Bug', priority: 'Medium', title: 'Fix login redirect after session expiry',
    jira: JIRA.review, assignee: 'Sam Ortiz', reporter: 'Jane Doe', sprint: 'Sprint 41', epic: 'Auth hardening',
    fixVersion: '2.14.0', estimate: '3 pts', created: '3 days ago', updated: '1 hour ago',
    labels: ['auth', 'regression'], components: ['api-client', 'web'],
    desc: [
      P('Users whose session expires while they are on a deep link are sent to /home after logging back in, instead of the page they were on. Support has seen it on three accounts this week.'),
      H('Steps to reproduce'),
      UL('Log in and open /orders/4821', 'Wait for the session to expire (15 min) or clear the session cookie', 'Click any action and log in again'),
      P('Expected: back on /orders/4821. Actual: /home.'),
      CODE('GET /login?next=%2Forders%2F4821   ->   302 /home'),
    ],
    ac: ['The deep link is preserved through login', 'Only same-origin next values are accepted', 'Existing login form tests still pass'],
    comments: [
      { n: 1, who: 'Jane Doe', at: 'yesterday 16:40', body: [P('Three customers hit this today. One lost an unsaved order form, so this is more than cosmetic.')] },
      { n: 2, who: 'Sam Ortiz', at: 'yesterday 17:05', body: [P('Root cause is in api-client: landing() ignores next. The fix is on two branches; the web side only passes the param through. @[you] can you take the review?')] },
      { n: 3, who: 'Jane Doe', at: 'today 09:12', body: [P('Please also check the open-redirect case. Security asked about it.')] },
    ],
    attachments: [{ name: 'session-expiry.mov', size: '2.1 MB' }, { name: 'network-trace.har', size: '340 KB' }],
    links: [{ rel: 'relates to', key: 'PROJ-127', title: 'Session cookie SameSite', status: 'Alpha Testing' }, { rel: 'is blocked by', key: 'PROJ-098', title: 'Refactor login module', status: 'Done' }],
    subtasks: [{ title: 'Add regression test', done: true }, { title: 'Update the runbook', done: false }],
    cands: { 'api-client': ['feature/PROJ-142-redirect'], web: ['feature/PROJ-142-web'] },
    local: { status: 'claimed' }, seenN: 1,
    checklist: [
      { text: 'Deep link survives login', done: false },
      { text: 'External next value is rejected', done: false },
      { text: 'Logout clears the session', done: false },
    ],
    prs: [
      { repo: 'api-client', id: 212, title: 'PROJ-142 Preserve next through login', src: 'feature/PROJ-142-redirect', dst: 'develop', updatedSeq: 1,
        reviewers: [{ name: 'Jane Doe', approved: false }, { name: ME, approved: false }],
        files: [
          { path: 'src/redirect.rs', adds: 9, dels: 1, lines: [
            L('ctx', 38, 38, 'use crate::user::User;'), L('ctx', 39, 39, ''),
            L('ctx', 40, 40, 'pub fn target(user: &User, next: Option<&str>) -> Url {'),
            L('del', 41, null, '    Url::parse("/home").unwrap()'),
            L('add', null, 41, '    match next.filter(|n| is_same_origin(n)) {'),
            L('add', null, 42, '        Some(n) => Url::parse(n).unwrap_or_else(|_| user.landing()),'),
            L('add', null, 43, '        None => user.landing(),'),
            L('add', null, 44, '    }'),
            L('ctx', 42, 45, '}'), L('ctx', 43, 46, ''),
            L('add', null, 47, 'fn is_same_origin(n: &str) -> bool {'),
            L('add', null, 48, '    n.starts_with(\'/\') && !n.starts_with("//")'),
            L('add', null, 49, '}'),
          ] },
          { path: 'tests/redirect_test.rs', adds: 14, dels: 0, lines: [
            L('ctx', 1, 1, 'use api_client::redirect::target;'), L('ctx', 2, 2, ''),
            L('add', null, 3, '#[test]'), L('add', null, 4, 'fn keeps_a_same_origin_next() {'),
            L('add', null, 5, '    let url = target(&user(), Some("/orders/4821"));'),
            L('add', null, 6, '    assert_eq!(url.path(), "/orders/4821");'), L('add', null, 7, '}'),
            L('add', null, 8, '#[test]'), L('add', null, 9, 'fn rejects_an_external_next() {'),
            L('add', null, 10, '    let url = target(&user(), Some("//evil.example/x"));'),
            L('add', null, 11, '    assert_eq!(url.path(), user().landing().path());'), L('add', null, 12, '}'),
          ] },
        ],
        threads: [{ id: 'th1', file: 'src/redirect.rs', line: 'n42', author: 'Jane Doe', text: 'Should we log when we reject a next value? Security asked for that.', resolved: true, replies: [] }],
      },
      { repo: 'web', id: 488, title: 'PROJ-142 Pass next to the login call', src: 'feature/PROJ-142-web', dst: 'develop', updatedSeq: 1,
        reviewers: [{ name: 'Jane Doe', approved: false }, { name: ME, approved: false }],
        files: [{ path: 'src/session.rs', adds: 4, dels: 2, lines: [
          L('ctx', 20, 20, 'pub fn login(req: &Request) -> Result<Session> {'),
          L('del', 21, null, '    let url = api::login(&req.credentials)?;'),
          L('add', null, 21, '    let next = req.query("next");'),
          L('add', null, 22, '    let url = api::login(&req.credentials, next.as_deref())?;'),
          L('ctx', 22, 23, '    Ok(Session::from(url))'), L('ctx', 23, 24, '}'),
        ] }],
        threads: [],
      },
    ],
  });

  add({
    key: 'PROJ-150', type: 'Story', priority: 'High', title: 'Fix header overflow on narrow screens',
    jira: JIRA.review, assignee: 'Priya Nair', reporter: 'Marta Lind', sprint: 'Sprint 41', epic: 'Mobile polish',
    fixVersion: '2.14.0', estimate: '2 pts', created: '2 days ago', updated: '3 hours ago',
    labels: ['ui', 'mobile'], components: ['web'],
    desc: [
      P('On viewports narrower than 380px the header menu wraps under the logo and pushes the page sideways. The search field is the culprit.'),
      H('Acceptance'), UL('No horizontal scroll at 320px', 'Menu stays reachable', 'Desktop layout unchanged'),
    ],
    ac: ['No horizontal scroll at 320px', 'Menu stays reachable', 'Desktop layout unchanged'],
    comments: [
      { n: 1, who: 'Marta Lind', at: '2 days ago', body: [P('Screenshot attached from an iPhone SE. It looks worse in landscape.')] },
      { n: 2, who: 'Priya Nair', at: 'yesterday 11:20', body: [P('Fixed with a flex-wrap and a shrinking search. Two branches exist because I restarted; the -old one can be ignored.')] },
    ],
    attachments: [{ name: 'iphone-se.png', size: '480 KB' }],
    links: [], subtasks: [{ title: 'Check landscape', done: true }],
    cands: { web: ['feature/PROJ-150-header', 'feature/PROJ-150-header-old'] },
    checklist: [{ text: 'No horizontal scroll at 320px', done: false }, { text: 'Menu reachable', done: false }],
    prs: [{ repo: 'web', id: 490, title: 'PROJ-150 Wrap the header on narrow screens', src: 'feature/PROJ-150-header', dst: 'develop', updatedSeq: 1,
      reviewers: [{ name: 'Marta Lind', approved: false }, { name: ME, approved: false }],
      files: [
        { path: 'src/header.css', adds: 6, dels: 2, lines: [
          L('ctx', 10, 10, '.header {'), L('del', 11, null, '  display: flex;'),
          L('add', null, 11, '  display: flex;'), L('add', null, 12, '  flex-wrap: wrap;'), L('add', null, 13, '  gap: 8px;'),
          L('ctx', 12, 14, '}'), L('ctx', 13, 15, ''),
          L('ctx', 14, 16, '.header .search {'), L('del', 15, null, '  width: 320px;'),
          L('add', null, 17, '  flex: 1 1 120px;'), L('add', null, 18, '  min-width: 0;'), L('ctx', 16, 19, '}'),
        ] },
        { path: 'src/Header.tsx', adds: 2, dels: 1, lines: [
          L('ctx', 8, 8, 'export function Header() {'), L('del', 9, null, '  return <nav className="header">'),
          L('add', null, 9, '  return <nav className="header" aria-label="Main">'), L('ctx', 10, 10, '    <Logo />'),
        ] },
      ], threads: [{ id: 'th2', file: 'src/header.css', line: 'n12', author: 'Marta Lind', text: 'flex-wrap breaks the logo alignment on tablets. Can we keep the logo on its own row?', resolved: false, replies: [] }] }],
  });

  add({
    key: 'PROJ-139', type: 'Bug', priority: 'Highest', title: 'Payment retry double-charges on timeout',
    jira: JIRA.review, assignee: 'Sam Ortiz', reporter: 'Omar Haddad', sprint: 'Hotfix', epic: 'Payments',
    fixVersion: '2.13.1', estimate: '2 pts', created: 'today 07:50', updated: '30 min ago',
    labels: ['payments', 'hotfix'], components: ['api-client', 'web'],
    desc: [
      P('When the payment provider times out, the client retries with a new idempotency key. If the first request actually succeeded the customer is charged twice.'),
      H('Impact'), UL('11 customers charged twice since Monday', 'Refunds are being handled manually'),
      P('Fix: reuse the idempotency key across retries. Targets the production branch because this is live.'),
    ],
    ac: ['A retry reuses the original idempotency key', 'A timed-out request is never re-sent with a new key'],
    comments: [
      { n: 1, who: 'Omar Haddad', at: 'today 07:55', body: [P('This is costing us real money. Fastest safe fix please.')] },
      { n: 2, who: 'Sam Ortiz', at: 'today 09:30', body: [P('Both PRs target master and main. Needs @[you] to review and test.')] },
    ],
    attachments: [{ name: 'double-charge-report.csv', size: '18 KB' }],
    links: [{ rel: 'is caused by', key: 'PROJ-071', title: 'Client retry policy', status: 'Done' }],
    subtasks: [],
    cands: { 'api-client': ['hotfix/PROJ-139-idempotency'], web: ['hotfix/PROJ-139-web'] },
    checklist: [{ text: 'Retry keeps the same key', done: false }, { text: 'No second charge on timeout', done: false }],
    prs: [
      { repo: 'api-client', id: 214, title: 'PROJ-139 Reuse idempotency key on retry', src: 'hotfix/PROJ-139-idempotency', dst: 'master', updatedSeq: 1,
        reviewers: [{ name: 'Omar Haddad', approved: false }, { name: ME, approved: false }],
        files: [{ path: 'src/payments.rs', adds: 5, dels: 3, lines: [
          L('ctx', 60, 60, 'fn charge(&self, req: &Charge) -> Result<Receipt> {'),
          L('del', 61, null, '    let key = Uuid::new_v4();'),
          L('add', null, 61, '    let key = req.idempotency_key.get_or_insert_with(Uuid::new_v4);'),
          L('ctx', 62, 62, '    self.post("/charges", req, key)'), L('ctx', 63, 63, '}'),
        ] }], threads: [] },
      { repo: 'web', id: 491, title: 'PROJ-139 Send a stable key from the checkout', src: 'hotfix/PROJ-139-web', dst: 'main', updatedSeq: 1,
        reviewers: [{ name: 'Omar Haddad', approved: false }, { name: ME, approved: false }],
        files: [{ path: 'src/checkout.ts', adds: 3, dels: 1, lines: [
          L('ctx', 14, 14, 'async function pay(order: Order) {'),
          L('del', 15, null, '  return api.charge(order);'),
          L('add', null, 15, '  order.key ??= crypto.randomUUID();'),
          L('add', null, 16, '  return api.charge(order);'), L('ctx', 16, 17, '}'),
        ] }], threads: [] },
    ],
  });

  add({
    key: 'PROJ-131', type: 'Task', priority: 'Low', title: 'Staging redirect loop',
    jira: JIRA.returned, assignee: 'Sam Ortiz', reporter: 'Jane Doe', sprint: 'Sprint 40', epic: 'Auth hardening',
    fixVersion: '2.13.0', estimate: '1 pt', created: '9 days ago', updated: '2 hours ago',
    labels: ['auth'], components: ['web'],
    desc: [P('Expired sessions loop between /login and /home on staging only.'), UL('Reproduce with a stale cookie', 'Compare staging and production cookie domains')],
    ac: ['No redirect loop with an expired cookie'],
    comments: [
      { n: 1, who: 'Sam Ortiz', at: '6 days ago', body: [P('Deployed to alpha, cookie domain fixed.')] },
      { n: 2, who: 'Jane Doe', at: 'today 10:05', body: [P('Sent back. @[you] can you re-check the redirect on staging? It still loops for expired sessions.')] },
    ],
    attachments: [], links: [{ rel: 'relates to', key: 'PROJ-142', title: 'Fix login redirect after session expiry', status: 'In Review' }], subtasks: [],
    cands: { web: ['feature/PROJ-131-cookie'] },
    local: { status: 'integrated' }, reviewed: true, reviewedSeq: 1, seenN: 1,
    merges: [{ repo: 'web', commit: '2b90cd3', at: '6 days ago' }],
    deploy: { web: { state: 'deployed', run: 401, since: -1e9 } },
    drafts: [{ id: 'd1', kind: 'comment', body: 'Deployed to alpha.', status: 'posted' }],
    prs: [{ repo: 'web', id: 470, title: 'PROJ-131 Scope the session cookie', src: 'feature/PROJ-131-cookie', dst: 'develop', updatedSeq: 1,
      reviewers: [{ name: 'Jane Doe', approved: false }, { name: ME, approved: false }],
      files: [{ path: 'src/cookie.ts', adds: 2, dels: 1, lines: [L('ctx', 3, 3, 'export const cookie = {'), L('del', 4, null, '  domain: ".acme.dev",'), L('add', null, 4, '  domain: "staging.acme.dev",'), L('ctx', 5, 5, '};')] }],
      threads: [] }],
  });

  add({
    key: 'PROJ-127', type: 'Story', priority: 'Medium', title: 'Session cookie SameSite',
    jira: JIRA.alpha, assignee: 'Priya Nair', reporter: 'Omar Haddad', sprint: 'Sprint 40', epic: 'Auth hardening',
    fixVersion: '2.13.0', estimate: '2 pts', created: '12 days ago', updated: 'yesterday',
    labels: ['auth', 'security'], components: ['api-client', 'web'],
    desc: [P('Set SameSite=Lax on the session cookie and document the cross-site flows that need None.'), UL('api-client sets the attribute', 'web forwards it on redirects')],
    ac: ['Cookie carries SameSite=Lax', 'Docs list the exceptions'],
    comments: [
      { n: 1, who: 'Priya Nair', at: 'yesterday 15:10', body: [P('Deployed to alpha.'), P('api-client: run #311 https://github.com/acme/api-client/actions/runs/311'), P('web: run #398 https://github.com/acme/web/actions/runs/398')] },
    ],
    attachments: [], links: [{ rel: 'relates to', key: 'PROJ-142', title: 'Fix login redirect after session expiry', status: 'In Review' }], subtasks: [],
    cands: { 'api-client': ['feature/PROJ-127-samesite'], web: ['feature/PROJ-127-samesite'] },
    local: { status: 'integrated' }, reviewed: true, reviewedSeq: 1, seenN: 1,
    merges: [{ repo: 'api-client', commit: '8c01d22', at: 'yesterday' }, { repo: 'web', commit: '44ab0f9', at: 'yesterday' }],
    deploy: { 'api-client': { state: 'deployed', run: 311, since: -1e9 }, web: { state: 'deployed', run: 398, since: -1e9 } },
    drafts: [{ id: 'd0', kind: 'comment', body: 'Deployed to alpha.', status: 'posted' }],
    prs: [
      { repo: 'api-client', id: 205, title: 'PROJ-127 SameSite=Lax', src: 'feature/PROJ-127-samesite', dst: 'develop', updatedSeq: 1, reviewers: [{ name: 'Omar Haddad', approved: true }, { name: ME, approved: false }],
        files: [{ path: 'src/cookie.rs', adds: 2, dels: 0, lines: [L('add', null, 12, '    .same_site(SameSite::Lax)')] }], threads: [] },
      { repo: 'web', id: 469, title: 'PROJ-127 Forward SameSite', src: 'feature/PROJ-127-samesite', dst: 'develop', updatedSeq: 1, reviewers: [{ name: 'Omar Haddad', approved: true }, { name: ME, approved: false }],
        files: [{ path: 'src/session.rs', adds: 3, dels: 1, lines: [L('ctx', 30, 30, 'fn store(c: Cookie) {'), L('add', null, 31, '    let c = c.same_site(Lax);')] }], threads: [] },
    ],
  });

  add({
    key: 'PROJ-160', type: 'Task', priority: 'Low', title: 'Update onboarding copy',
    jira: JIRA.review, assignee: 'Marta Lind', reporter: 'Marta Lind', sprint: 'Sprint 41', epic: 'Docs',
    fixVersion: '2.14.0', estimate: '1 pt', created: 'yesterday', updated: 'yesterday',
    labels: ['docs'], components: ['docs'],
    desc: [P('Replace the old welcome text with the new wording from marketing.')], ac: ['New wording in the getting started page'],
    comments: [], attachments: [], links: [], subtasks: [],
    cands: { docs: ['docs/PROJ-160-onboarding'] },
    checklist: [{ text: 'Page reads correctly', done: false }],
    prs: [{ repo: 'docs', id: 58, title: 'PROJ-160 Onboarding copy', src: 'docs/PROJ-160-onboarding', dst: 'master', updatedSeq: 1,
      reviewers: [{ name: 'Marta Lind', approved: false }, { name: ME, approved: false }],
      files: [{ path: 'getting-started.md', adds: 4, dels: 3, lines: [L('ctx', 1, 1, '# Getting started'), L('del', 2, null, 'Welcome aboard.'), L('add', null, 2, 'Welcome to Acme. Here is how to get going in ten minutes.')] }], threads: [] }],
  });

  add({
    key: 'PROJ-163', type: 'Story', priority: 'Medium', title: 'Export button missing on the reports page',
    jira: JIRA.review, assignee: 'Priya Nair', reporter: 'Jane Doe', sprint: 'Sprint 41', epic: 'Reporting',
    fixVersion: '2.14.0', estimate: '2 pts', created: 'today 08:50', updated: 'today 09:20',
    labels: ['reports'], components: ['web'],
    desc: [P('The CSV export button disappeared from the reports page after the toolbar refactor. It should sit next to the date filter again.')], ac: ['Export button visible on /reports', 'Export respects the date filter'],
    comments: [{ n: 1, who: 'Priya Nair', at: 'today 09:20', body: [P('Branch is pushed, I will open the PR after lunch.')] }],
    cands: { web: ['feature/PROJ-163-export'] },
    checklist: [{ text: 'Button visible next to the date filter', done: false }, { text: 'Export respects the filter', done: false }],
    prs: [],
  });

  return tickets;
}

/* Tickets that "arrive" on later fake syncs. */
function syncScript() {
  return [
    { text: 'PROJ-155 appeared in the Review column', apply(S) {
      S.tickets.push(Object.assign(seedTickets()[0], {
        key: 'PROJ-155', type: 'Bug', priority: 'High', title: 'Toast overlaps the footer on iPad',
        jira: JIRA.review, assignee: 'Priya Nair', reporter: 'Marta Lind', sprint: 'Sprint 41', epic: 'Mobile polish',
        labels: ['ui'], components: ['web'], created: 'today 10:40', updated: 'just now',
        desc: [P('On iPad in portrait the success toast covers the footer links until it is dismissed.')], ac: ['Toast never covers the footer'],
        comments: [{ n: 1, who: 'Marta Lind', at: 'today 10:41', body: [P('Seen on iPad Air. @[you] you had this pattern in the header ticket, thoughts?')] }],
        attachments: [], links: [], subtasks: [], seenN: 0, local: null, reviewed: false, merges: [], deploy: {}, drafts: [],
        cands: { web: ['feature/PROJ-155-toast'] }, checklist: [{ text: 'Toast clear of the footer', done: false }],
        prs: [{ repo: 'web', id: 495, title: 'PROJ-155 Lift the toast above the footer', src: 'feature/PROJ-155-toast', dst: 'develop', updatedSeq: 1,
          reviewers: [{ name: 'Marta Lind', approved: false }, { name: ME, approved: false }],
          files: [{ path: 'src/toast.css', adds: 2, dels: 1, lines: [L('ctx', 5, 5, '.toast {'), L('del', 6, null, '  bottom: 0;'), L('add', null, 6, '  bottom: 64px;'), L('ctx', 7, 7, '}')] }], threads: [] }],
      }));
    } },
    { text: 'PROJ-163: a pull request was opened in web', apply(S) { openFakePrs(S.tickets.find((x) => x.key === 'PROJ-163')); } },
    { text: 'PROJ-150 got a new comment that mentions you', apply(S) {
      const t = S.tickets.find((x) => x.key === 'PROJ-150');
      t.comments.push({ n: 3, who: 'Marta Lind', at: 'just now', body: [P('@[you] can you check the header on iPad landscape too?')] });
    } },
  ];
}

/* what a freshly opened PR looks like (used by sync arrivals and the Simulate panel) */
function openFakePrs(t) {
  if (!t) return;
  for (const repo of Object.keys(t.cands)) {
    if (t.prs.some((p) => p.repo === repo)) continue;
    const src = (t.link && t.link[repo]) || t.cands[repo][0];
    t.prs.push({ repo, id: 500 + t.prs.length + Math.floor(Math.random() * 40), title: t.key + ' ' + t.title, src, dst: REPOS[repo].base, updatedSeq: 1,
      reviewers: [{ name: t.reporter, approved: false }, { name: ME, approved: false }],
      files: [{ path: 'src/reports.rs', adds: 3, dels: 1, lines: [L('ctx', 12, 12, 'fn toolbar() -> Toolbar {'), L('del', 13, null, '    Toolbar::new()'), L('add', null, 13, '    Toolbar::new()'), L('add', null, 14, '        .with_filter(date_filter())'), L('add', null, 15, '        .with_export(csv_export())'), L('ctx', 14, 16, '}')] }],
      threads: [] });
  }
}
