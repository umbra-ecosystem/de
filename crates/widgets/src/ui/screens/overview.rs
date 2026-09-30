//! Overview tab: what Jira says, the repos and their branches, the discussion. Full-width bands separated by hairlines.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::widgets::*;
use crate::vm::*;

/// The repo table runs flush: every row has its hairline from edge to edge.
fn repos_table(ui: &Ui, rows: &[RepoRowVm]) -> Div {
    let pal = &ui.pal;
    let col = |w: f32| div().flex_none().w(px(w)).px_2();
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .px_4()
                .py_1()
                .border_t_1()
                .border_b_1()
                .border_color(pal.border.opacity(0.6))
                .text_xs()
                .text_color(pal.faint)
                .child(col(110.0).child("Repo"))
                .child(div().flex_1().px_2().child("Branch"))
                .child(col(150.0).child("PR"))
                .child(col(180.0).child("Deploy")),
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
                            .child(name.to_string()),
                    )
                    .when(*manual, |d| d.child(faint(pal, "chosen"))),
                BranchCell::Ambiguous(choices) => warn_box(
                    pal,
                    Tone::Warn,
                    [
                        div()
                            .child(format!("{} branches match. Choose one:", choices.len()))
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
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(pal.border.opacity(0.4))
                .when(!r.touched, |d| d.opacity(0.6))
                .child(
                    col(110.0)
                        .font_family(ui.mono.clone())
                        .text_sm()
                        .child(r.repo.to_string()),
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

/// A comment is a row, not a card: avatar, who and when, then the text; a hairline below.
fn comment(ui: &Ui, cx: &App, c: &CommentVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .gap_3()
        .px_6()
        .py_3()
        .border_t_1()
        .border_color(pal.border.opacity(0.4))
        .child(avatar(&c.who))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .child(div().child(c.who.clone()))
                        .child(faint(pal, c.at.clone()))
                        .when(c.is_new, |d| {
                            d.child(div().text_xs().text_color(pal.accent).child("new"))
                        }),
                )
                .child(blocks(pal, cx, &c.body)),
        )
}

fn checks(ui: &Ui, items: Vec<(bool, String)>) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_1()
        .children(items.into_iter().map(|(done, text)| {
            div()
                .flex()
                .gap_2()
                .text_sm()
                .child(div().flex_none().text_color(pal.muted).child(if done {
                    "☑"
                } else {
                    "☐"
                }))
                .child(div().flex_1().min_w_0().child(text))
        }))
}

pub fn overview(ui: &Ui, cx: &App, inputs: &Inputs, o: &OverviewVm) -> Div {
    let pal = &ui.pal;
    let mut bands: Vec<(String, AnyElement)> = Vec::new();
    bands.push((
        "Description".to_string(),
        pad(blocks(pal, cx, &o.description)).into_any_element(),
    ));
    bands.push((
        "Acceptance criteria".to_string(),
        pad(checks(
            ui,
            o.acceptance.iter().map(|a| (false, a.clone())).collect(),
        ))
        .into_any_element(),
    ));
    if !o.subtasks.is_empty() {
        bands.push((
            "Subtasks".to_string(),
            pad(checks(ui, o.subtasks.clone())).into_any_element(),
        ));
    }
    if !o.attachments.is_empty() {
        bands.push((
            "Attachments".to_string(),
            pad(div()
                .flex()
                .flex_col()
                .gap_2()
                .children(o.attachments.iter().map(|(n, size)| {
                    div()
                        .flex()
                        .items_baseline()
                        .gap_2()
                        .text_sm()
                        .child(div().child(n.clone()))
                        .child(faint(pal, size.clone()))
                })))
            .into_any_element(),
        ));
    }
    bands.push((
        "Repos and pull requests".to_string(),
        repos_table(ui, &o.repos).into_any_element(),
    ));
    bands.push((
        format!("Comments ({})", o.comments.len()),
        div()
            .flex()
            .flex_col()
            .children(o.comments.iter().map(|c| comment(ui, cx, c)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .px_6()
                    .py_3()
                    .border_t_1()
                    .border_color(pal.border.opacity(0.4))
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
            )
            .into_any_element(),
    ));
    div().flex().flex_col().children(
        bands
            .into_iter()
            .enumerate()
            .map(|(i, (title, content))| band(pal, i == 0, title, content)),
    )
}
