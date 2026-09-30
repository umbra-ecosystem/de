# de prototype

A clickable prototype of the menubar app and main window. **Everything is fake**: tickets, comments, PRs,
pipelines, sync and providers are in-memory data in `data.js`; nothing touches Jira, Bitbucket, git or the disk.
It exists to judge the flows and layout before any GUI is built.

Open `index.html` in a browser. Use the **Simulate** button (top bar) to make the outside world misbehave:
offline, a signed-out tool, a `uat` conflict, an overlay leak, `uat` moving before a push, a failing deploy,
new mentions, sign-off, a returned ticket.

## Files

| File | Role |
|---|---|
| `data.js` | Fake tickets, repos, PRs with diffs, and the scripted "arrivals" that fake syncs deliver |
| `logic.js` | A small in-memory stand-in for `de-core`: lifecycle actions, fake pipelines, audit log, and a simplified copy of the next-action rules |
| `views_a.js` `views_b.js` `views_c.js` | Shell (fixed left sidebar, right context sidebar, top bar, now bar), screens, and sheets, click handlers, boot |
| `style.css` | Low-fidelity wireframe styling, light and dark |
| `build.py` | Bundles everything into the single `index.html` |
| `test.js`, `smoke.js` | Dependency-free Node tests: the lifecycle and rules (`node test.js`), and every screen plus the click handlers (`node smoke.js`) |

Rebuild after editing: `python3 build.py`. Run tests: `node test.js && node smoke.js`.

## Fidelity

The rules in `logic.js` are a **simplification** of the real engine in `crates/core/src/next/` (same rule names,
bands and stable ids; fewer inputs). The real core is the source of truth: if they disagree, the prototype is wrong.
The screens and flows are described in `../gui.md`; the design ideas the prototype demonstrates are listed on its
"Change ideas" screen.
