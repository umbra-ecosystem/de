# de-widgets

The view layer of the de GUI, built on `gpui-kit`, separated from state.

- `vm`: view models. Plain data: what a screen shows and the `Intent`s it may emit.
- `store::Store`: the seam to the data. `sim::Sim` implements it in memory (a port of `docs/design/prototype`); `de-app` will implement it over the engine.
- `session::Session`: UI state on top of a store. No GPUI; the interaction model is unit-tested.
- `ui`: stateless GPUI widgets and screens, plus `ui::run()` which opens the showcase. The window chrome is a `TitleBar` (window controls, jump-to and the window-level toggles) above a `DockArea` (navigation left, the screen in the middle, details right; resizable and collapsible).

This crate has no engine, data or SQLite dependency by design. New UI is built here and shown in the showcase before `de-app` uses it.

```
# DE_SHOWCASE_ROUTE=ticket:PROJ-142:review DE_SHOWCASE_THEME=light opens a specific screen
cargo run -p de-widgets --bin showcase
cargo test -p de-widgets
```

## Scope versus the prototype

Ported: Next (ranking, reasons, dismiss/snooze/undo), ticket lists, the ticket screen with Overview, Review (unified/split diff, per-hunk viewed, inline comments, since-your-review, uat conflict/overlap banner), Test, Ship (prepare, pre-push checks, typed-key push, runs, re-run, announce, approve) and Timeline; On uat, Workspace, Audit, Settings, command palette, preview tabs, needs-attention panel, toasts with undo, all confirm/baseline/busy/one-ticket sheets, and the Simulate panel.

Not ported: the responsive/mobile layout, `j`/`k`/`Enter` list keys, and the prototype's automatic repo-fetch queue (only stale and external repo locks are simulated).
