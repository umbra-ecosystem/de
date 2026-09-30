//! Review tab: the pull request's own diff, per hunk "viewed", inline comments, and the writes that need a confirm.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::widgets::*;
use crate::vm::*;

fn thread(ui: &Ui, t: &ThreadVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_1()
        .ml_12()
        .my_1()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(pal.border)
        .bg(pal.raised)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(avatar(pal, &t.initials))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t.author.clone()),
                )
                .when(t.mine, |d| {
                    d.child(pill(pal, &Badge::new("you", Tone::Accent)))
                })
                .child(pill(
                    pal,
                    &if t.resolved {
                        Badge::new("resolved", Tone::Ok)
                    } else {
                        Badge::new("unresolved", Tone::Warn)
                    },
                )),
        )
        .child(div().text_sm().child(t.text.clone()))
}

fn composer(ui: &Ui, inputs: &Inputs, path: &str, anchor: &str) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_2()
        .ml_12()
        .my_1()
        .p_2()
        .rounded_md()
        .border_1()
        .border_color(pal.accent.opacity(0.6))
        .child(faint(
            pal,
            format!("Comment on {path} line {}", anchor.get(1..).unwrap_or("")),
        ))
        .child(inputs.area(&Field::Inline))
        .child(
            div()
                .flex()
                .gap_2()
                .child(button(
                    ui,
                    &Btn::new("Comment…", Intent::Submit(Field::Inline)).primary(),
                ))
                .child(button_ghost(ui, &Btn::new("Cancel", Intent::InlineCancel))),
        )
}

fn num(pal: &crate::ui::theme::Pal, n: Option<u32>) -> Div {
    div()
        .flex_none()
        .w_9()
        .text_right()
        .pr_2()
        .text_xs()
        .text_color(pal.faint)
        .child(n.map_or(String::new(), |n| n.to_string()))
}

fn kind_bg(pal: &crate::ui::theme::Pal, k: LineKind) -> Hsla {
    match k {
        LineKind::Add => pal.add_bg,
        LineKind::Del => pal.del_bg,
        LineKind::Context => gpui_kit::transparent_black(),
    }
}

fn sign(k: LineKind) -> &'static str {
    match k {
        LineKind::Add => "+",
        LineKind::Del => "−",
        LineKind::Context => " ",
    }
}

#[allow(clippy::too_many_arguments)]
fn plus(
    ui: &Ui,
    key: &str,
    pr: u32,
    path: &str,
    anchor: &str,
    group: &SharedString,
) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("plus-{group}")))
        .flex_none()
        .w_5()
        .text_center()
        .rounded_sm()
        .cursor_pointer()
        .text_color(ui.pal.accent)
        .opacity(0.0)
        .group_hover(group.clone(), |s| s.opacity(1.0))
        .hover(|s| s.bg(ui.pal.accent.opacity(0.2)))
        .on_click(ui.on_click(Intent::InlineOpen {
            key: key.to_string(),
            pr,
            file: path.to_string(),
            line: anchor.to_string(),
        }))
        .child("+")
}

fn unified_row(ui: &Ui, inputs: &Inputs, r: &ReviewVm, hunk: usize, row: &DiffRow) -> Div {
    let pal = &ui.pal;
    let DiffRow::Line {
        line,
        threads,
        composer: comp,
    } = row
    else {
        return div();
    };
    let group = SharedString::from(format!("ln-{hunk}-{}", line.anchor));
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .group(group.clone())
                .flex()
                .items_start()
                .bg(kind_bg(pal, line.kind))
                .child(num(pal, line.old))
                .child(num(pal, line.new))
                .child(
                    div()
                        .flex_none()
                        .w_4()
                        .text_color(pal.muted)
                        .child(sign(line.kind)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(ui.mono.clone())
                        .text_sm()
                        .whitespace_nowrap()
                        .child(line.text.clone()),
                )
                .child(plus(
                    ui,
                    &r.key,
                    r.pr_id,
                    &r.file_path,
                    &line.anchor,
                    &group,
                )),
        )
        .children(threads.iter().map(|t| thread(ui, t)))
        .when(*comp, |d| {
            d.child(composer(ui, inputs, &r.file_path, &line.anchor))
        })
}

fn split_cell(ui: &Ui, l: &Option<DiffLineVm>, left: bool) -> Div {
    let pal = &ui.pal;
    match l {
        Some(l) => div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_start()
            .bg(kind_bg(pal, l.kind))
            .child(num(pal, if left { l.old } else { l.new }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(ui.mono.clone())
                    .text_sm()
                    .whitespace_nowrap()
                    .child(l.text.clone()),
            ),
        None => div().flex_1().min_w_0().bg(pal.border.opacity(0.15)),
    }
}

fn split_row(ui: &Ui, inputs: &Inputs, r: &ReviewVm, hunk: usize, row: &DiffRow) -> Div {
    let pal = &ui.pal;
    let DiffRow::Pair {
        left,
        right,
        threads,
        composer: comp,
    } = row
    else {
        return div();
    };
    let anchor = right
        .as_ref()
        .or(left.as_ref())
        .map(|l| l.anchor.clone())
        .unwrap_or_default();
    let group = SharedString::from(format!("sp-{hunk}-{anchor}"));
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .group(group.clone())
                .flex()
                .child(split_cell(ui, left, true))
                .child(div().w_px().bg(pal.border))
                .child(split_cell(ui, right, false))
                .child(plus(ui, &r.key, r.pr_id, &r.file_path, &anchor, &group)),
        )
        .children(threads.iter().map(|t| thread(ui, t)))
        .when(*comp, |d| {
            d.child(composer(ui, inputs, &r.file_path, &anchor))
        })
}

fn hunk(ui: &Ui, inputs: &Inputs, r: &ReviewVm, h: &HunkVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .py_1()
                .bg(pal.border.opacity(if h.viewed { 0.2 } else { 0.4 }))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .text_xs()
                        .font_family(ui.mono.clone())
                        .text_color(pal.faint)
                        .child(format!("hunk {} of {}", h.index + 1, h.of))
                        .child(div().text_color(pal.ok).child(format!("+{}", h.adds)))
                        .child(div().text_color(pal.bad).child(format!("−{}", h.dels))),
                )
                .child(check_row(
                    ui,
                    "viewed",
                    "Viewed",
                    h.viewed,
                    h.toggle.clone(),
                )),
        )
        .children(h.rows.iter().map(|row| match row {
            DiffRow::Line { .. } => unified_row(ui, inputs, r, h.index, row),
            DiffRow::Pair { .. } => split_row(ui, inputs, r, h.index, row),
        }))
}

pub fn review(ui: &Ui, _cx: &App, inputs: &Inputs, r: &ReviewVm) -> Div {
    let pal = &ui.pal;
    if r.empty {
        return div()
            .flex()
            .flex_col()
            .gap_3()
            .children(r.uat.iter().map(|b| banner(ui, b)))
            .child(empty_state(pal, "No pull requests for this ticket."));
    }
    let tree = div()
        .flex()
        .flex_col()
        .gap_1()
        .w(px(270.0))
        .flex_none()
        .children(r.groups.iter().map(|g| {
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .font_family(ui.mono.clone())
                        .text_xs()
                        .text_color(pal.faint)
                        .child(g.label.clone()),
                )
                .children(g.files.iter().map(|f| {
                    div()
                        .id(SharedString::from(format!("file-{}-{}", g.label, f.path)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(if f.selected {
                            pal.accent.opacity(0.18)
                        } else {
                            gpui_kit::transparent_black()
                        })
                        .hover(|s| s.bg(pal.hover))
                        .on_click(ui.on_click(f.select.clone()))
                        .child(
                            div()
                                .min_w_0()
                                .font_family(ui.mono.clone())
                                .text_xs()
                                .child(f.path.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .gap_1()
                                .text_xs()
                                .child(div().text_color(pal.ok).child(format!("+{}", f.adds)))
                                .child(div().text_color(pal.bad).child(format!("−{}", f.dels)))
                                .when_some(f.progress.clone(), |d, p| d.child(faint(pal, p))),
                        )
                }))
        }))
        .child(faint(pal, format!("{} inline comments", r.thread_count)));

    let diff = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap_2()
        .child(
            div()
                .font_family(ui.mono.clone())
                .text_sm()
                .text_color(pal.muted)
                .child(r.file_path.clone()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .rounded_md()
                .border_1()
                .border_color(pal.border)
                .overflow_hidden()
                .children(r.hunks.iter().map(|h| hunk(ui, inputs, r, h))),
        );

    div()
        .flex()
        .flex_col()
        .gap_3()
        .children(r.uat.iter().map(|b| banner(ui, b)))
        .children(r.stale.iter().map(|b| banner(ui, b)))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(div().flex().gap_2().when_some(
                    r.since_toggle.clone(),
                    |d, (a, b, since)| {
                        d.child(chip(ui, a.label.clone(), since, a.intent.clone()))
                            .child(chip(ui, b.label.clone(), !since, b.intent.clone()))
                    },
                ))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(chip(
                            ui,
                            "Unified",
                            r.diff_mode == DiffMode::Unified,
                            Intent::SetDiffMode(DiffMode::Unified),
                        ))
                        .child(chip(
                            ui,
                            "Split",
                            r.diff_mode == DiffMode::Split,
                            Intent::SetDiffMode(DiffMode::Split),
                        )),
                ),
        )
        .when_some(r.pr.clone(), |d, p| {
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(p.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .text_color(pal.muted)
                            .child(p.source.clone())
                            .child("→")
                            .child(pill(pal, &p.dest)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_xs()
                            .child("Reviewers:")
                            .child(pills(pal, &p.reviewers)),
                    ),
            )
        })
        .when(r.prs.len() > 1, |d| {
            d.child(
                div().flex().gap_2().children(
                    r.prs
                        .iter()
                        .map(|(label, on, intent)| chip(ui, label.clone(), *on, intent.clone())),
                ),
            )
        })
        .child(div().flex().gap_4().items_start().child(tree).child(diff))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .pt_2()
                .border_t_1()
                .border_color(pal.border)
                .child(button(ui, &r.mark_reviewed))
                .child(button(ui, &r.request_changes))
                .child(button(ui, &r.approve))
                .child(faint(pal, r.note.clone())),
        )
}
