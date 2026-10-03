//! Ship tab: one vertical stepper, each step shows its state and the one thing to do next.

use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::widgets::*;
use crate::vm::*;

fn state_icon(pal: &crate::ui::theme::Pal, state: StepState) -> AnyElement {
    match state {
        StepState::Done => Icon::new(IconName::CircleCheck)
            .small()
            .text_color(pal.ok)
            .into_any_element(),
        StepState::Bad => Icon::new(IconName::TriangleAlert)
            .small()
            .text_color(pal.bad)
            .into_any_element(),
        StepState::Now => div()
            .size_4()
            .flex()
            .items_center()
            .justify_center()
            .child(dot(pal.accent))
            .into_any_element(),
        StepState::Todo => div()
            .size_4()
            .flex()
            .items_center()
            .justify_center()
            .child(dot(pal.faint))
            .into_any_element(),
    }
}

/// One stage of shipping: a status icon and a title, its content beneath, hairlines between stages.
fn step(ui: &Ui, index: usize, title: &str, state: StepState, body: Div) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_2()
        .w_full()
        .px_4()
        .py_3()
        .when(index > 0, |d| {
            d.border_t_1().border_color(pal.border.opacity(0.6))
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(state_icon(pal, state))
                .child(
                    div()
                        .text_sm()
                        .text_color(if state == StepState::Todo {
                            pal.muted
                        } else {
                            pal.fg
                        })
                        .child(title.to_string()),
                ),
        )
        .child(div().pl_6().child(body))
}

/// A soft notice (overlap, new commits, uat moved): one quiet line with a coloured icon, never a box.
fn quiet(ui: &Ui, tone: Tone, text: impl Into<SharedString>) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .items_start()
        .gap_2()
        .py_1()
        .text_xs()
        .text_color(pal.muted)
        .child(
            div().flex_none().pt_0p5().child(
                Icon::new(IconName::TriangleAlert)
                    .small()
                    .text_color(pal.tone(tone)),
            ),
        )
        .child(div().flex_1().min_w_0().child(text.into()))
}

fn row(ui: &Ui) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .py_1p5()
        .flex_wrap()
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
        IntegrateBody::Inactive(msg) => faint(pal, msg.clone()),
        IntegrateBody::Pushed { rows, new_commits } => div()
            .flex()
            .flex_col()
            .child(div().flex().flex_col().children(rows.iter().map(|(repo, commit, at)| {
                row(ui)
                    .child(repo_cell(ui, repo))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .font_family(ui.mono.clone())
                            .text_xs()
                            .text_color(pal.faint)
                            .child(commit.clone())
                            .child(copy_button(ui, commit.clone())),
                    )
                    .child(faint(pal, at.clone()))
            })))
            .when(*new_commits, |d| {
                d.child(quiet(
                    ui,
                    Tone::Warn,
                    "New commits on the ticket branches are not on uat. Activate the ticket again to re-merge.",
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
                    .child(faint(pal, "Overlay guard passed (by SHA); no overlay in the worktree.")),
            })
            .when(!prepush.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(faint(pal, "Before you push"))
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
        PrepRow::Ready {
            repo,
            commits,
            files,
            uat_before,
            merge,
            overlaps,
        } => div()
            .flex()
            .flex_col()
            .child(
                row(ui)
                    .child(repo_cell(ui, repo))
                    .child(pill(pal, &Badge::new("merge ready", Tone::Ok)))
                    .child(
                        div()
                            .text_sm()
                            .child(format!("+{commits} commits · {files} files")),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .font_family(ui.mono.clone())
                            .text_xs()
                            .text_color(pal.faint)
                            .child(format!("uat {uat_before} → {merge}"))
                            .child(copy_button(ui, merge.clone())),
                    ),
            )
            .when(!overlaps.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(overlaps.iter().map(|o| {
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(quiet(
                                    ui,
                                    Tone::Warn,
                                    format!(
                                        "Also changed by {} on uat. Not a conflict; check the combined behaviour.",
                                        o.text
                                    ),
                                ))
                                .when_some(o.tickets.first().cloned(), |d, k| {
                                    d.child(button_ghost(
                                        ui,
                                        &Btn::new(
                                            k.to_string(),
                                            Intent::go_ticket(k, TicketTab::Overview),
                                        ),
                                    ))
                                })
                        })),
                )
            }),
        PrepRow::AlreadyPushed { repo, commit } => row(ui)
            .child(repo_cell(ui, repo))
            .child(pill(pal, &Badge::new("already pushed", Tone::Ok)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .font_family(ui.mono.clone())
                    .text_xs()
                    .text_color(pal.faint)
                    .child(commit.clone())
                    .child(copy_button(ui, commit.clone())),
            ),
        PrepRow::Conflict {
            repo,
            with,
            files,
            comment,
            sent,
        } => row(ui)
            .child(repo_cell(ui, repo))
            .child(pill(pal, &Badge::new("conflict", Tone::Bad)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(format!(
                        "Conflicts with {with} in {files}. The merge was aborted; nothing was pushed."
                    ))
                    .child(button_ghost(
                        ui,
                        &Btn::new(
                            with.to_string(),
                            Intent::go_ticket(with.clone(), TicketTab::Overview),
                        ),
                    )),
            )
            .when_some(comment.clone(), |d, a| d.child(button(ui, &a)))
            .when_some(sent.clone(), |d, at| {
                d.child(pill(
                    pal,
                    &Badge::new(format!("comment sent {at}"), Tone::Ok),
                ))
            }),
        PrepRow::Blocked { repo, reason } => row(ui)
            .child(repo_cell(ui, repo))
            .child(pill(pal, &Badge::new("blocked", Tone::Bad)))
            .child(div().text_sm().flex_1().min_w_0().child(reason.clone())),
    }
}

fn runs(ui: &Ui, cx: &App, s: &ShipVm) -> Div {
    let pal = &ui.pal;
    if s.runs.is_empty() {
        return faint(pal, "Workflow runs appear after the push.");
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
                        .child(if r.chip.tone == Tone::Ok {
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(faint(pal, format!("deployed to alpha · run #{}", r.chip.run)))
                                .when_some(r.chip.url.clone(), |d, url| {
                                    d.child(open_button(ui, url, "Open run"))
                                })
                        } else {
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(deploy_chip(pal, &r.chip))
                                .when_some(r.chip.url.clone(), |d, url| {
                                    d.child(open_button(ui, url, "Open run"))
                                })
                        })
                        .when_some(r.rerun.clone(), |d, a| d.child(button(ui, &a))),
                )
                .when_some(r.failure.clone(), |d, (step, run, log)| {
                    d.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .my_1()
                            .pl_3()
                            .border_l_2()
                            .border_color(pal.bad)
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(
                                        div().text_sm().child(format!("Failed at step “{step}”")),
                                    )
                                    .child(faint(pal, format!("run #{run}"))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(code_block(pal, cx, log.clone()))
                                    .child(
                                        div().flex().child(copy_button(ui, log)),
                                    ),
                            ),
                    )
                })
        }))
        .when_some(s.uat_moved.clone(), |d, m| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(quiet(ui, Tone::Warn, m.text()))
                    .child(button_ghost(
                        ui,
                        &Btn::new(
                            m.ticket.to_string(),
                            Intent::go_ticket(m.ticket.clone(), TicketTab::Overview),
                        ),
                    )),
            )
        })
}

fn announce(ui: &Ui, cx: &App, inputs: &Inputs, key: &TicketKey, b: &AnnounceBody) -> Div {
    let pal = &ui.pal;
    match b {
        AnnounceBody::Unavailable => faint(pal, "Available when every touched repo is deployed."),
        AnnounceBody::Compose(a) => div().flex().child(button(ui, a)),
        AnnounceBody::Draft { id, post, .. } => div()
            .flex()
            .flex_col()
            .gap_2()
            .child(inputs.area(&Field::Draft {
                key: key.clone(),
                id: id.clone(),
            }))
            .child(div().flex().child(button(ui, post))),
        AnnounceBody::Posted {
            text,
            moved,
            transition,
        } => div()
            .flex()
            .flex_col()
            .gap_2()
            .child(code_block(pal, cx, text.clone()))
            .child(div().flex().child(copy_button(ui, text.clone())))
            .child(faint(
                pal,
                match moved {
                    Some(b) => format!("Posted to Jira · now {}", b.text),
                    None => "Posted to Jira".to_string(),
                },
            ))
            .when_some(transition.clone(), |d, a| {
                d.child(div().flex().child(button(ui, &a)))
            }),
    }
}

pub fn ship(ui: &Ui, cx: &App, inputs: &Inputs, s: &ShipVm) -> Div {
    let pal = &ui.pal;
    let after = div()
        .flex()
        .flex_col()
        .gap_2()
        .child(faint(pal, s.after_text.clone()))
        .when_some(s.approve_all.clone(), |d, a| {
            d.child(div().flex().child(button(ui, &a)))
        })
        .children(s.after.iter().map(|r| {
            row(ui)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .w(px(150.0))
                        .flex_none()
                        .font_family(ui.mono.clone())
                        .text_sm()
                        .child(r.label.clone())
                        .when_some(r.url.clone(), |d, url| {
                            d.child(open_button(ui, url, "Open pull request"))
                        }),
                )
                .child(pill(pal, &r.badge))
                .when_some(r.approve.clone(), |d, a| d.child(button(ui, &a)))
        }));
    div()
        .flex()
        .flex_col()
        .child(step(
            ui,
            0,
            "Integrate to uat",
            s.integrate_state,
            integrate(ui, &s.integrate),
        ))
        .child(step(ui, 1, "GitHub Actions", s.runs_state, runs(ui, cx, s)))
        .child(step(
            ui,
            2,
            "Announce",
            s.announce_state,
            announce(ui, cx, inputs, &s.key, &s.announce),
        ))
        .child(step(ui, 3, "After alpha", s.after_state, after))
}
