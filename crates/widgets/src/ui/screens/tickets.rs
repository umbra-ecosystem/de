//! Ticket lists: the review pool, what is in hand, awaiting alpha, returned, done.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::Ui;
use crate::ui::widgets::*;
use crate::vm::*;

fn cell(w: f32) -> Div {
    div().flex_none().w(px(w)).px_2()
}

fn header(ui: &Ui) -> Div {
    let pal = &ui.pal;
    let h = |w: f32, t: &str| {
        cell(w)
            .text_xs()
            .text_color(pal.faint)
            .child(t.to_uppercase())
    };
    div()
        .flex()
        .items_center()
        .py_1()
        .border_b_1()
        .border_color(pal.border)
        .child(h(90.0, "Key"))
        .child(h(84.0, "Priority"))
        .child(
            div()
                .flex_1()
                .px_2()
                .text_xs()
                .text_color(pal.faint)
                .child("TITLE"),
        )
        .child(h(110.0, "Jira"))
        .child(h(96.0, "Local"))
        .child(h(150.0, "Repos"))
        .child(h(230.0, "Flags"))
}

pub fn ticket_row(ui: &Ui, r: &TicketRowVm) -> Stateful<Div> {
    let pal = &ui.pal;
    let intent = Intent::go_ticket(&r.key, TicketTab::Overview);
    div()
        .id(SharedString::from(format!("row-{}", r.key)))
        .flex()
        .items_center()
        .py_2()
        .border_b_1()
        .border_color(pal.border.opacity(0.5))
        .cursor_pointer()
        .hover(|s| s.bg(pal.hover))
        .on_click(ui.on_click(intent))
        .child(cell(90.0).child(key_text(ui, &r.key)))
        .child(cell(84.0).child(pill(pal, &r.priority)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(r.title.clone()),
                        )
                        .when(r.hotfix, |d| d.child(hotfix_pill(pal))),
                )
                .child(faint(pal, r.sub.clone())),
        )
        .child(cell(110.0).child(pill(pal, &r.jira)))
        .child(cell(96.0).child(match &r.local {
            Some(b) => pill(pal, b),
            None => div().text_xs().text_color(pal.faint).child("unclaimed"),
        }))
        .child(
            cell(150.0)
                .flex()
                .flex_wrap()
                .gap_1()
                .children(r.repos.iter().map(|x| mono_chip(ui, x))),
        )
        .child(cell(230.0).child(pills(pal, &r.flags)))
}

fn mono_chip(ui: &Ui, text: &str) -> Div {
    div()
        .text_xs()
        .font_family(ui.mono.clone())
        .text_color(ui.pal.muted)
        .child(text.to_string())
}

fn table(ui: &Ui, rows: &[TicketRowVm]) -> Div {
    div()
        .flex()
        .flex_col()
        .child(header(ui))
        .children(rows.iter().map(|r| ticket_row(ui, r)))
}

pub fn tickets(ui: &Ui, vm: &TicketListVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(heading(vm.group.label()))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .children(vm.tabs.iter().map(|(g, n)| {
                    chip(
                        ui,
                        format!("{} {n}", g.label()),
                        *g == vm.group,
                        Intent::go_tickets(*g),
                    )
                })),
        )
        .child(if vm.sections.is_empty() {
            empty_state(pal, "No tickets here.")
        } else {
            div()
                .flex()
                .flex_col()
                .gap_4()
                .children(vm.sections.iter().map(|s| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .when_some(s.heading.clone(), |d, h| {
                            d.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(h))
                        })
                        .child(table(ui, &s.rows))
                }))
        })
}
