//! Overview tab: what Jira says, the repos and their branches, comments, your notes.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::widgets::*;
use crate::vm::*;

fn repos_table(ui: &Ui, rows: &[RepoRowVm]) -> Div {
    let pal = &ui.pal;
    let col = |w: f32| div().flex_none().w(px(w)).px_2();
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .py_1()
                .border_b_1()
                .border_color(pal.border)
                .text_xs()
                .text_color(pal.faint)
                .child(col(110.0).child("REPO"))
                .child(div().flex_1().px_2().child("BRANCH"))
                .child(col(150.0).child("PR"))
                .child(col(180.0).child("DEPLOY")),
        )
        .children(rows.iter().map(|r| {
            let branch = match &r.branch {
                BranchCell::Chosen { name, manual } => div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .font_family(ui.mono.clone())
                            .text_sm()
                            .child(name.clone()),
                    )
                    .when(*manual, |d| {
                        d.child(pill(pal, &Badge::new("chosen", Tone::Neutral)))
                    }),
                BranchCell::Ambiguous(choices) => warn_box(
                    pal,
                    Tone::Warn,
                    [
                        div()
                            .child(format!("⚠ {} branches match. Choose one:", choices.len()))
                            .into_any_element(),
                        buttons(ui, choices).into_any_element(),
                    ],
                ),
                BranchCell::Baseline(b) => div()
                    .text_sm()
                    .text_color(pal.faint)
                    .font_family(ui.mono.clone())
                    .child(format!("{b} (baseline)")),
                BranchCell::None => div().text_color(pal.faint).child("–"),
            };
            div()
                .flex()
                .items_center()
                .py_2()
                .border_b_1()
                .border_color(pal.border.opacity(0.4))
                .when(!r.touched, |d| d.opacity(0.6))
                .child(
                    col(110.0)
                        .font_family(ui.mono.clone())
                        .text_sm()
                        .child(r.repo.clone()),
                )
                .child(div().flex_1().px_2().child(branch))
                .child(
                    col(150.0)
                        .text_sm()
                        .child(r.pr.clone().unwrap_or_else(|| "–".to_string())),
                )
                .child(col(180.0).child(match &r.deploy {
                    Some(c) => deploy_chip(pal, c),
                    None => faint(
                        pal,
                        if r.touched {
                            "not pushed"
                        } else {
                            "not touched"
                        },
                    ),
                }))
        }))
}

fn comment(ui: &Ui, cx: &App, c: &CommentVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(if c.is_new {
            pal.accent.opacity(0.6)
        } else {
            pal.border
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(avatar(pal, &c.initials))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_sm()
                                .child(c.who.clone()),
                        )
                        .child(faint(pal, c.at.clone())),
                )
                .when(c.is_new, |d| {
                    d.child(pill(pal, &Badge::new("new", Tone::Accent)))
                }),
        )
        .child(blocks(pal, cx, &c.body))
}

pub fn overview(ui: &Ui, cx: &App, inputs: &Inputs, o: &OverviewVm) -> Div {
    let pal = &ui.pal;
    let left = div()
        .flex()
        .flex_col()
        .gap_5()
        .flex_1()
        .min_w_0()
        .child(section(pal, "Description").child(blocks(pal, cx, &o.description)))
        .child(
            section(pal, "Acceptance criteria").child(div().flex().flex_col().gap_1().children(
                o.acceptance.iter().map(|a| {
                    div()
                        .flex()
                        .gap_2()
                        .text_sm()
                        .child(div().text_color(pal.muted).child("☐"))
                        .child(a.clone())
                }),
            )),
        )
        .when(!o.subtasks.is_empty(), |d| {
            d.child(
                section(pal, "Subtasks").child(div().flex().flex_col().gap_1().children(
                    o.subtasks.iter().map(|(done, t)| {
                        div()
                            .flex()
                            .gap_2()
                            .text_sm()
                            .child(div().text_color(pal.muted).child(if *done {
                                "☑"
                            } else {
                                "☐"
                            }))
                            .child(t.clone())
                    }),
                )),
            )
        })
        .child(section(pal, "Repos and pull requests").child(repos_table(ui, &o.repos)))
        .child(
            section(pal, format!("Comments ({}, from Jira)", o.comments.len()))
                .child(if o.comments.is_empty() {
                    muted(pal, "No comments yet.")
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(o.comments.iter().map(|c| comment(ui, cx, c)))
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(inputs.area(&Field::Comment(o.key.clone())))
                        .child(
                            div().flex().justify_end().child(button(
                                ui,
                                &Btn::new(
                                    "Post to Jira…",
                                    Intent::Submit(Field::Comment(o.key.clone())),
                                )
                                .primary(),
                            )),
                        ),
                ),
        );
    let right = div()
        .flex()
        .flex_col()
        .gap_5()
        .w(px(300.0))
        .flex_none()
        .when(!o.attachments.is_empty(), |d| {
            d.child(
                section(pal, "Attachments").child(div().flex().flex_col().gap_2().children(
                    o.attachments.iter().map(|(n, size)| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_color(pal.muted).child("▤"))
                            .child(
                                div()
                                    .child(div().text_sm().child(n.clone()))
                                    .child(faint(pal, size.clone())),
                            )
                    }),
                )),
            )
        })
        .child(
            section(pal, "Your notes")
                .child(inputs.area(&Field::Notes(o.key.clone())))
                .child(faint(pal, "Private, saved locally.")),
        );
    div().flex().gap_6().items_start().child(left).child(right)
}
