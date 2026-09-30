//! Review tab: the pull request's own diff. Two panes (files, diff) between a slim toolbar and an action bar,
//! filling the screen; the tab strip above stays put.

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::theme::Pal;
use crate::ui::widgets::*;
use crate::vm::*;

const GUTTER: f32 = 40.0;

/* ------------------------------ diff lines ------------------------------ */

fn num(pal: &Pal, n: Option<u32>) -> Div {
    div()
        .flex_none()
        .w(px(GUTTER))
        .pr_2()
        .text_right()
        .text_xs()
        .text_color(pal.faint)
        .child(n.map_or(String::new(), |n| n.to_string()))
}

fn kind_bg(pal: &Pal, k: LineKind) -> Hsla {
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

/// The "+" that appears at the right of a line on hover and starts an inline comment.
fn plus(
    ui: &Ui,
    key: &TicketKey,
    pr: PrNumber,
    path: &str,
    anchor: LineAnchor,
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
        .hover(|s| s.bg(ui.pal.hover))
        .on_click(ui.on_click(Intent::InlineOpen {
            key: key.clone(),
            pr,
            file: path.to_string(),
            line: anchor,
        }))
        .child("+")
}

/// An inline discussion: a rule on the left and the text, no box.
fn thread(ui: &Ui, t: &ThreadVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .gap_3()
        .ml(px(GUTTER * 2.0))
        .my_1p5()
        .pl_3()
        .border_l_2()
        .border_color(if t.resolved { pal.border } else { pal.warn })
        .child(avatar(&t.author))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .child(div().child(t.author.clone()))
                        .when(t.mine, |d| d.child(faint(pal, "you")))
                        .child(faint(
                            pal,
                            if t.resolved { "resolved" } else { "unresolved" },
                        )),
                )
                .child(div().text_sm().child(t.text.clone())),
        )
}

fn composer(ui: &Ui, inputs: &Inputs, path: &str, anchor: LineAnchor) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_2()
        .ml(px(GUTTER * 2.0))
        .my_1p5()
        .pl_3()
        .border_l_2()
        .border_color(pal.accent)
        .child(faint(pal, format!("Comment on {path}, {anchor}")))
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
    let group = SharedString::from(format!("ln-{hunk}-{:?}", line.anchor));
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
                        .text_xs()
                        .whitespace_nowrap()
                        .child(line.text.clone()),
                )
                .child(plus(ui, &r.key, r.pr_id, &r.file_path, line.anchor, &group)),
        )
        .children(threads.iter().map(|t| thread(ui, t)))
        .when(*comp, |d| {
            d.child(composer(ui, inputs, &r.file_path, line.anchor))
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
                    .text_xs()
                    .whitespace_nowrap()
                    .child(l.text.clone()),
            ),
        None => div().flex_1().min_w_0().bg(pal.border.opacity(0.12)),
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
        .map_or(LineAnchor::New(0), |l| l.anchor);
    let group = SharedString::from(format!("sp-{hunk}-{anchor:?}"));
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
                .child(plus(ui, &r.key, r.pr_id, &r.file_path, anchor, &group)),
        )
        .children(threads.iter().map(|t| thread(ui, t)))
        .when(*comp, |d| {
            d.child(composer(ui, inputs, &r.file_path, anchor))
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
                .h_7()
                .px_3()
                .bg(pal.border.opacity(if h.viewed { 0.15 } else { 0.3 }))
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
                .child(
                    Checkbox::new(SharedString::from(format!("viewed-{}", h.index)))
                        .label("Viewed")
                        .checked(h.viewed)
                        .small()
                        .on_click(ui.on_toggle(h.toggle.clone())),
                ),
        )
        .children(h.rows.iter().map(|row| match row {
            DiffRow::Line { .. } => unified_row(ui, inputs, r, h.index, row),
            DiffRow::Pair { .. } => split_row(ui, inputs, r, h.index, row),
        }))
}

/* ------------------------------ panes ------------------------------ */

/// `src/` faint, `payments.rs` regular: the file name is what you scan for.
fn file_name(pal: &Pal, path: &str) -> Div {
    let (dir, name) = path.rsplit_once('/').map_or(("", path), |(d, n)| (d, n));
    div()
        .flex()
        .min_w_0()
        .text_sm()
        .when(!dir.is_empty(), |d| {
            d.child(div().text_color(pal.faint).child(format!("{dir}/")))
        })
        .child(div().truncate().child(name.to_string()))
}

fn tree(ui: &Ui, r: &ReviewVm) -> Stateful<Div> {
    let pal = &ui.pal;
    div()
        .id("review-files")
        .flex()
        .flex_col()
        .w(px(264.0))
        .flex_none()
        .h_full()
        .border_r_1()
        .border_color(pal.border)
        .overflow_y_scroll()
        .children(r.groups.iter().map(|g| {
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .px_3()
                        .pt_3()
                        .pb_1()
                        .text_xs()
                        .text_color(pal.muted)
                        .child(g.label.clone()),
                )
                .children(g.files.iter().map(|f| {
                    div()
                        .id(SharedString::from(format!("file-{}-{}", g.label, f.path)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .h_7()
                        .px_3()
                        .cursor_pointer()
                        .bg(if f.selected {
                            pal.selected
                        } else {
                            gpui_kit::transparent_black()
                        })
                        .hover(|s| s.bg(pal.hover))
                        .on_click(ui.on_click(f.select.clone()))
                        .child(file_name(pal, &f.path))
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap_1p5()
                                .text_xs()
                                .font_family(ui.mono.clone())
                                .when(f.viewed_all, |d| {
                                    d.child(
                                        Icon::new(IconName::CircleCheck).small().text_color(pal.ok),
                                    )
                                })
                                .child(div().text_color(pal.ok).child(format!("+{}", f.adds)))
                                .child(div().text_color(pal.bad).child(format!("−{}", f.dels))),
                        )
                }))
        }))
        .child(
            div()
                .px_3()
                .py_3()
                .child(faint(pal, format!("{} inline comments", r.thread_count))),
        )
}

fn diff_pane(ui: &Ui, inputs: &Inputs, r: &ReviewVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            div()
                .flex()
                .items_center()
                .flex_none()
                .h_9()
                .px_3()
                .border_b_1()
                .border_color(pal.border)
                .text_xs()
                .font_family(ui.mono.clone())
                .text_color(pal.muted)
                .child(r.file_path.clone()),
        )
        .child(
            div()
                .id("review-diff")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .children(r.hunks.iter().map(|h| hunk(ui, inputs, r, h))),
                ),
        )
}

/// One line above the panes: which pull request, where it goes, who reviews it, how to show the diff.
/// "Overlaps uat" as a quiet marker in the toolbar; what overlaps is in its tooltip.
fn overlap_hint(ui: &Ui, b: &BannerVm) -> Stateful<Div> {
    let pal = &ui.pal;
    let tip = b.lines.join("\n");
    div()
        .id("uat-overlap")
        .flex()
        .items_center()
        .gap_1()
        .flex_none()
        .text_xs()
        .text_color(pal.muted)
        .child(
            Icon::new(IconName::TriangleAlert)
                .small()
                .text_color(pal.tone(b.tone)),
        )
        .child("Overlaps uat")
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
}

fn toolbar(ui: &Ui, r: &ReviewVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .items_center()
        .gap_3()
        .flex_none()
        .h_10()
        .px_4()
        .border_b_1()
        .border_color(pal.border)
        .children(if r.prs.len() > 1 {
            Some(
                div().flex().gap_1().children(
                    r.prs
                        .iter()
                        .map(|(label, on, intent)| chip(ui, label.clone(), *on, intent.clone())),
                ),
            )
        } else {
            None
        })
        .when_some(r.pr.clone(), |d, p| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .text_xs()
                    .font_family(ui.mono.clone())
                    .text_color(pal.muted)
                    .child(div().truncate().child(p.source.clone()))
                    .child(
                        Icon::new(IconName::ArrowRight)
                            .small()
                            .text_color(pal.faint),
                    )
                    .child(pill(pal, &p.dest)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .flex_none()
                    .children(p.reviewers.iter().map(|(name, approved)| {
                        div()
                            .opacity(if *approved { 1.0 } else { 0.55 })
                            .child(avatar(name))
                    }))
                    .child(faint(
                        pal,
                        format!(
                            "{}/{} approved",
                            p.reviewers.iter().filter(|(_, a)| *a).count(),
                            p.reviewers.len()
                        ),
                    )),
            )
        })
        .child(div().flex_1())
        .when_some(r.uat.clone().filter(|b| b.tone != Tone::Bad), |d, b| {
            d.child(overlap_hint(ui, &b))
        })
        .when_some(r.since_toggle.clone(), |d, (a, b, since)| {
            d.child(
                div()
                    .flex()
                    .gap_1()
                    .child(chip(ui, a.label.clone(), since, a.intent.clone()))
                    .child(chip(ui, b.label.clone(), !since, b.intent.clone())),
            )
        })
        .child(
            div()
                .flex()
                .gap_1()
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
        )
}

fn action_bar(ui: &Ui, r: &ReviewVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .items_center()
        .gap_2()
        .flex_none()
        .px_4()
        .py_2()
        .border_t_1()
        .border_color(pal.border)
        .child(div().flex_1().child(faint(pal, r.note.clone())))
        .child(button(ui, &r.request_changes))
        .child(button(ui, &r.approve))
        .child(button(ui, &r.mark_reviewed))
}

/// A notice above the panes. A conflict is a box; anything softer (overlap, new commits) is one quiet line.
fn notice(ui: &Ui, b: &BannerVm) -> AnyElement {
    let pal = &ui.pal;
    if b.tone == Tone::Bad {
        return banner(ui, b).into_any_element();
    }
    div()
        .flex()
        .items_center()
        .gap_2()
        .text_sm()
        .child(
            Icon::new(IconName::TriangleAlert)
                .small()
                .text_color(pal.tone(b.tone)),
        )
        .child(div().child(b.title.trim_start_matches(['⚠', ' ']).to_string()))
        .children(b.lines.iter().map(|l| faint(pal, l.clone())))
        .when(!b.actions.is_empty(), |d| d.child(buttons(ui, &b.actions)))
        .into_any_element()
}

/// The whole tab. It fills its parent and scrolls inside its panes, not as a page.
pub fn review(ui: &Ui, _cx: &App, inputs: &Inputs, r: &ReviewVm) -> Div {
    let notices = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_2()
        .px_4()
        .py_2()
        // Only a conflict interrupts the page; a mere overlap is a hint in the toolbar.
        .children(
            r.uat
                .iter()
                .filter(|b| b.tone == Tone::Bad)
                .map(|b| notice(ui, b)),
        )
        .children(r.stale.iter().map(|b| notice(ui, b)));
    if r.empty {
        return div().flex().flex_col().size_full().child(notices).child(
            div()
                .px_4()
                .py_4()
                .child(empty_panel(
                    ui,
                    &EmptyVm::new(
                        "No pull requests",
                        "Open a pull request from one of this ticket's branches; it shows up here after the next sync.",
                    )
                    .action(Btn::new("Sync now", Intent::Do(Command::Sync))),
                )),
        );
    }
    div()
        .flex()
        .flex_col()
        .size_full()
        .when(
            r.uat.as_ref().is_some_and(|b| b.tone == Tone::Bad) || r.stale.is_some(),
            |d| d.child(notices),
        )
        .child(toolbar(ui, r))
        .child(
            div()
                .flex()
                .flex_1()
                .min_h_0()
                .child(tree(ui, r))
                .child(diff_pane(ui, inputs, r)),
        )
        .child(action_bar(ui, r))
}
