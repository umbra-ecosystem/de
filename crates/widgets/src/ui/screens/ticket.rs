//! The ticket screen: a persistent header (status, banners, stepper, tabs) above one of five tabs.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{overview, review, ship, test_tab};
use crate::ui::ctx::{Inputs, Ui};
use crate::ui::widgets::*;
use crate::vm::*;

fn head(ui: &Ui, h: &TicketHeadVm, tab: TicketTab) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(key_text(ui, &h.key))
                        .child(pill(pal, &h.jira))
                        .when_some(h.local.clone(), |d, b| d.child(pill(pal, &b)))
                        .when(h.hotfix, |d| d.child(hotfix_pill(pal)))
                        .when_some(h.uat_flag.clone(), |d, b| d.child(pill(pal, &b))),
                )
                .child(buttons(ui, &h.actions)),
        )
        .child(
            div()
                .text_xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(h.title.clone()),
        )
        .children(h.banners.iter().map(|b| banner(ui, b)))
        .child(stepper(pal, &h.stepper))
        .child(
            div()
                .flex()
                .border_b_1()
                .border_color(pal.border)
                .children(h.tabs.iter().map(|(t, dot_on)| {
                    seg_tab(
                        ui,
                        t.label(),
                        *t == tab,
                        *dot_on,
                        Intent::go_ticket(&h.key, *t),
                    )
                })),
        )
}

pub fn ticket(
    ui: &Ui,
    cx: &App,
    inputs: &Inputs,
    h: &TicketHeadVm,
    tab: TicketTab,
    body: &TicketBody,
    comment: &str,
    checklist_add: &str,
) -> Div {
    let pal = &ui.pal;
    let _ = (comment, checklist_add);
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(head(ui, h, tab))
        .child(match body {
            TicketBody::Overview(o) => overview::overview(ui, cx, inputs, o),
            TicketBody::Review(r) => review::review(ui, cx, inputs, r),
            TicketBody::Test(t) => test_tab::test(ui, cx, inputs, &h.key, t),
            TicketBody::Ship(s) => ship::ship(ui, cx, inputs, s),
            TicketBody::Timeline(rows) => timeline(ui, &h.key, rows),
        })
        .child(div().h_8())
        .text_color(pal.fg)
}

fn audit_table(ui: &Ui, rows: &[AuditRow], show_ticket: bool) -> Div {
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
                .child(col(60.0).child("TIME"))
                .child(col(220.0).child("ACTION"))
                .child(col(100.0).child("REPO"))
                .child(col(80.0).child("OUTCOME"))
                .child(div().flex_1().px_2().child("DETAILS")),
        )
        .children(rows.iter().map(|a| {
            div()
                .flex()
                .items_start()
                .py_1()
                .border_b_1()
                .border_color(pal.border.opacity(0.4))
                .text_sm()
                .child(col(60.0).font_family(ui.mono.clone()).child(a.at.clone()))
                .child(
                    col(220.0)
                        .font_family(ui.mono.clone())
                        .child(a.action.clone()),
                )
                .child(
                    col(100.0)
                        .font_family(ui.mono.clone())
                        .child(a.repo.clone()),
                )
                .child(col(80.0).child(pill(pal, &a.outcome)))
                .child(
                    div()
                        .flex_1()
                        .px_2()
                        .flex()
                        .gap_2()
                        .when(show_ticket, |d| {
                            d.when_some(a.ticket.clone(), |d, k| d.child(key_text(ui, &k)))
                        })
                        .child(div().text_xs().child(a.details.clone())),
                )
        }))
}

pub fn audit_rows(ui: &Ui, rows: &[AuditRow], show_ticket: bool) -> Div {
    audit_table(ui, rows, show_ticket)
}

fn timeline(ui: &Ui, key: &str, rows: &[AuditRow]) -> Div {
    let pal = &ui.pal;
    section(pal, format!("Audit log for {key}")).child(if rows.is_empty() {
        empty_state(pal, "No activity yet for this ticket.")
    } else {
        audit_table(ui, rows, false)
    })
}
