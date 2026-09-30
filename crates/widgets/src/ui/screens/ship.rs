//! Ship tab: one vertical stepper, each step shows its state and the one thing to do next.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::widgets::*;
use crate::vm::*;

fn step(ui: &Ui, n: usize, title: &str, state: StepState, body: Div) -> Div {
    let pal = &ui.pal;
    div().flex().gap_3().child(step_badge(pal, n, state)).child(
        div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_w_0()
            .pb_4()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.to_string()),
            )
            .child(body),
    )
}

fn row(ui: &Ui) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .py_1p5()
        .border_b_1()
        .border_color(ui.pal.border.opacity(0.4))
}

fn repo_cell(ui: &Ui, repo: &str) -> Div {
    div()
        .w(px(100.0))
        .flex_none()
        .font_family(ui.mono.clone())
        .text_sm()
        .child(repo.to_string())
}

fn integrate(ui: &Ui, b: &IntegrateBody) -> Div {
    let pal = &ui.pal;
    match b {
        IntegrateBody::Inactive(msg) => muted(pal, msg.clone()),
        IntegrateBody::Pushed { rows, new_commits } => div()
            .flex()
            .flex_col()
            .child(div().flex().flex_col().children(rows.iter().map(|(repo, commit, at)| {
                row(ui)
                    .child(repo_cell(ui, repo))
                    .child(pill(pal, &Badge::new("on uat", Tone::Ok)))
                    .child(div().font_family(ui.mono.clone()).text_xs().text_color(pal.faint).child(commit.clone()))
                    .child(faint(pal, at.clone()))
            })))
            .when(*new_commits, |d| {
                d.child(warn_box(
                    pal,
                    Tone::Warn,
                    [div()
                        .child("⚠ New commits on the ticket branches are not on uat. Activate the ticket again to re-merge.")
                        .into_any_element()],
                ))
            }),
        IntegrateBody::Active { prep, prepush, prepare, push } => div()
            .flex()
            .flex_col()
            .gap_2()
            .child(match prep {
                None => muted(
                    pal,
                    "Preparing merges each touched repo into a temporary worktree of uat and checks it. It pushes nothing.",
                ),
                Some(rows) => div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(rows.iter().map(|r| prep_row(ui, r)))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .pt_1()
                            .child(pill(pal, &Badge::new("overlay guard ✓ by SHA", Tone::Ok)))
                            .child(pill(pal, &Badge::new("no overlay in worktree ✓", Tone::Ok))),
                    ),
            })
            .when(!prepush.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(pal.border)
                        .child(faint(pal, "BEFORE YOU PUSH"))
                        .children(prepush.iter().map(|g| {
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_4()
                                .child(repo_cell(ui, &g.repo))
                                .children(g.items.iter().map(|(text, done, intent)| {
                                    check_row(ui, "pre", text, *done, intent.clone())
                                }))
                        })),
                )
            })
            .child(div().flex().gap_2().child(button(ui, prepare)).child(button(ui, push))),
    }
}

fn prep_row(ui: &Ui, r: &PrepRow) -> Div {
    let pal = &ui.pal;
    match r {
        PrepRow::Ready { repo, commits, files, uat_before, merge, overlaps } => div()
            .flex()
            .flex_col()
            .child(
                row(ui)
                    .child(repo_cell(ui, repo))
                    .child(pill(pal, &Badge::new("merge ready", Tone::Ok)))
                    .child(div().text_sm().child(format!("+{commits} commits · {files} files")))
                    .child(
                        div()
                            .font_family(ui.mono.clone())
                            .text_xs()
                            .text_color(pal.faint)
                            .child(format!("uat {uat_before} → {merge}")),
                    ),
            )
            .when(!overlaps.is_empty(), |d| {
                d.child(warn_box(
                    pal,
                    Tone::Warn,
                    [div()
                        .child(format!(
                            "⚠ Overlaps with {} already on uat. Not a conflict, but check the combined behaviour.",
                            overlaps.join(", ")
                        ))
                        .into_any_element()],
                ))
            }),
        PrepRow::AlreadyPushed { repo, commit } => row(ui)
            .child(repo_cell(ui, repo))
            .child(pill(pal, &Badge::new("already pushed", Tone::Ok)))
            .child(div().font_family(ui.mono.clone()).text_xs().text_color(pal.faint).child(commit.clone())),
        PrepRow::Conflict { repo, with, files, comment, sent } => row(ui)
            .child(repo_cell(ui, repo))
            .child(pill(pal, &Badge::new("conflict", Tone::Bad)))
            .child(div().text_sm().flex_1().child(format!(
                "Conflicts with {with} in {files}. The merge was aborted; nothing was pushed."
            )))
            .when_some(comment.clone(), |d, a| d.child(button(ui, &a)))
            .when_some(sent.clone(), |d, at| {
                d.child(pill(pal, &Badge::new(format!("comment sent {at}"), Tone::Ok)))
            }),
        PrepRow::Blocked { repo, reason } => row(ui)
            .child(repo_cell(ui, repo))
            .child(pill(pal, &Badge::new("blocked", Tone::Bad)))
            .child(div().text_sm().child(reason.clone())),
    }
}

fn runs(ui: &Ui, cx: &App, s: &ShipVm) -> Div {
    let pal = &ui.pal;
    if s.runs.is_empty() {
        return muted(pal, "Workflow runs appear after the push.");
    }
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(s.runs.iter().map(|r| {
            div()
                .flex()
                .flex_col()
                .child(
                    row(ui)
                        .child(repo_cell(ui, &r.repo))
                        .child(deploy_chip(pal, &r.chip))
                        .when_some(r.rerun.clone(), |d, a| d.child(button(ui, &a))),
                )
                .when_some(r.failure.clone(), |d, (step, run, log)| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .my_1()
                            .p_2()
                            .rounded_md()
                            .border_1()
                            .border_color(pal.bad.opacity(0.5))
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(
                                        div()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_sm()
                                            .child(format!("Failed at step “{step}”")),
                                    )
                                    .child(faint(pal, format!("run #{run}"))),
                            )
                            .child(code_block(pal, cx, log)),
                    )
                })
        }))
        .when_some(s.uat_moved.clone(), |d, m| {
            d.child(warn_box(
                pal,
                Tone::Warn,
                [div().child(format!("⚠ {m}")).into_any_element()],
            ))
        })
}

fn announce(ui: &Ui, cx: &App, inputs: &Inputs, key: &str, b: &AnnounceBody) -> Div {
    let pal = &ui.pal;
    match b {
        AnnounceBody::Unavailable => muted(pal, "Available when every touched repo is deployed."),
        AnnounceBody::Compose(a) => div().child(button(ui, a)),
        AnnounceBody::Draft { id, post, .. } => div()
            .flex()
            .flex_col()
            .gap_2()
            .child(inputs.area(&Field::Draft {
                key: key.to_string(),
                id: id.clone(),
            }))
            .child(div().child(button(ui, post))),
        AnnounceBody::Posted {
            text,
            moved,
            transition,
        } => div()
            .flex()
            .flex_col()
            .gap_2()
            .child(code_block(pal, cx, text.clone()))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(pill(pal, &Badge::new("posted to Jira", Tone::Ok)))
                    .when_some(moved.clone(), |d, b| d.child(pill(pal, &b))),
            )
            .when_some(transition.clone(), |d, a| {
                d.child(div().child(button(ui, &a)))
            }),
    }
}

pub fn ship(ui: &Ui, cx: &App, inputs: &Inputs, s: &ShipVm) -> Div {
    let pal = &ui.pal;
    let after = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(muted(pal, s.after_text.clone()))
        .when_some(s.approve_all.clone(), |d, a| {
            d.child(div().child(button(ui, &a)))
        })
        .children(s.after.iter().map(|r| {
            row(ui)
                .child(
                    div()
                        .w(px(150.0))
                        .flex_none()
                        .font_family(ui.mono.clone())
                        .text_sm()
                        .child(r.label.clone()),
                )
                .child(pill(pal, &r.badge))
                .when_some(r.approve.clone(), |d, a| d.child(button(ui, &a)))
        }));
    div()
        .flex()
        .flex_col()
        .child(step(
            ui,
            1,
            "Integrate to uat",
            s.integrate_state,
            integrate(ui, &s.integrate),
        ))
        .child(step(ui, 2, "GitHub Actions", s.runs_state, runs(ui, cx, s)))
        .child(step(
            ui,
            3,
            "Announce",
            s.announce_state,
            announce(ui, cx, inputs, &s.key, &s.announce),
        ))
        .child(step(ui, 4, "After alpha", s.after_state, after))
}
