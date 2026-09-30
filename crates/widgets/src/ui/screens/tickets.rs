//! Ticket lists: the review pool, what is in hand, awaiting alpha, returned, done. A uniform list of fixed-height rows.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::Ui;
use crate::ui::widgets::*;
use crate::vm::*;

const ROW_H: f32 = 52.0;

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
        .flex_none()
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
        .child(h(100.0, "Jira"))
        .child(h(92.0, "Local"))
        .child(h(120.0, "Repos"))
        .child(h(180.0, "Flags"))
}

/// One ticket row.
pub fn ticket_row(ui: &Ui, r: &TicketRowVm) -> Stateful<Div> {
    let pal = &ui.pal;
    let intent = Intent::go_ticket(r.key.clone(), TicketTab::Overview);
    div()
        .id(SharedString::from(format!("row-{}", r.key)))
        .flex()
        .items_center()
        .w_full()
        .h(px(ROW_H))
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
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(r.title.clone()),
                        )
                        .when(r.hotfix, |d| d.child(hotfix_pill(pal))),
                )
                .child(div().truncate().child(faint(pal, r.sub.clone()))),
        )
        .child(cell(100.0).child(pill(pal, &r.jira)))
        .child(
            cell(92.0).child(match &r.local {
                Some(b) => pill(pal, b).into_any_element(),
                None => div()
                    .text_xs()
                    .text_color(pal.faint)
                    .child("unclaimed")
                    .into_any_element(),
            }),
        )
        .child(
            cell(120.0)
                .flex()
                .gap_1()
                .overflow_hidden()
                .children(r.repos.iter().map(|x| mono_chip(ui, x))),
        )
        .child(
            cell(180.0)
                .flex()
                .gap_1()
                .overflow_hidden()
                .children(r.flags.iter().map(|b| pill(pal, b))),
        )
}

fn mono_chip(ui: &Ui, text: &str) -> Div {
    div()
        .text_xs()
        .font_family(ui.mono.clone())
        .text_color(ui.pal.muted)
        .child(text.to_string())
}

/// What the list shows, flattened so every item has the same height.
enum Item {
    Heading(String),
    Ticket(TicketRowVm),
}

pub fn tickets(ui: &Ui, vm: &TicketListVm) -> AnyElement {
    let pal = &ui.pal;
    let items: Vec<Item> = vm
        .sections
        .iter()
        .flat_map(|s| {
            s.heading
                .iter()
                .map(|h| Item::Heading(h.clone()))
                .chain(s.rows.iter().cloned().map(Item::Ticket))
                .collect::<Vec<_>>()
        })
        .collect();
    let head = div()
        .flex()
        .flex_col()
        .flex_none()
        .gap_3()
        .pb_2()
        .child(heading(vm.group.label()))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(vm.tabs.iter().map(|(g, n)| {
                    chip(
                        ui,
                        format!("{} {n}", g.label()),
                        *g == vm.group,
                        Intent::go_tickets(*g),
                    )
                })),
        );
    let body = if items.is_empty() {
        empty_state(pal, "No tickets here.").into_any_element()
    } else {
        let items = Rc::new(items);
        let ui = ui.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header(&ui))
            .child(
                uniform_list("ticket-list", items.len(), move |range, _, _| {
                    range
                        .map(|i| match &items[i] {
                            Item::Heading(h) => div()
                                .flex()
                                .items_end()
                                .h(px(ROW_H))
                                .px_2()
                                .pb_1()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(h.clone())
                                .into_any_element(),
                            Item::Ticket(r) => ticket_row(&ui, r).into_any_element(),
                        })
                        .collect::<Vec<_>>()
                })
                .w_full()
                .flex_1(),
            )
            .into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(head)
        .child(body)
        .into_any_element()
}
