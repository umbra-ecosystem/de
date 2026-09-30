//! The ticket screen: a persistent header (status, banners, stepper, tabs) above one of five tabs.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::{overview, review, ship, test_tab};
use crate::ui::ctx::{Inputs, Ui};
use crate::ui::widgets::*;
use crate::vm::*;

/// The part of the screen that never scrolls: the ticket's identity and its tabs.
fn head(ui: &Ui, h: &TicketHeadVm, tab: TicketTab) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .flex_none()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .px_4()
                .pt_4()
                .pb_2()
                // Two lines: what it is and what state it is in, then its title (wrapping) beside the actions.
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
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
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .gap_3()
                                .child(div().flex_1().min_w_0().text_lg().child(h.title.clone()))
                                .child(div().flex_none().child(buttons(ui, &h.actions))),
                        ),
                )
                .child(stepper(pal, &h.stepper)),
        )
        // The tab bar is full width (its underline and the review toolbar's line meet the panel edges).
        .child(tab_bar(
            ui,
            "ticket-tabs",
            h.tabs
                .iter()
                .map(|(t, dot_on)| (t.label().to_string(), *dot_on))
                .collect(),
            h.tabs.iter().position(|(t, _)| *t == tab).unwrap_or(0),
            h.tabs
                .iter()
                .map(|(t, _)| Intent::go_ticket(h.key.clone(), *t))
                .collect(),
        ))
}

/// A ticket: a fixed head (identity, tabs) above a body. The body scrolls, except on Review, whose panes scroll
/// on their own, so the tabs are always where you left them.
#[allow(clippy::too_many_arguments)]
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
    let notices = div()
        .flex()
        .flex_col()
        .gap_2()
        .children(h.banners.iter().map(|b| banner(ui, b)));
    let content = match body {
        TicketBody::Review(r) => div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .when(!h.banners.is_empty(), |d| {
                d.child(div().px_4().py_2().child(notices))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(review::review(ui, cx, inputs, r)),
            )
            .into_any_element(),
        other => div()
            .id("ticket-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when(!h.banners.is_empty(), |d| {
                        // No bottom padding: the first band below has its own top padding.
                        d.child(div().px_4().pt_3().child(notices))
                    })
                    .child(match other {
                        TicketBody::Overview(o) => overview::overview(ui, cx, inputs, o),
                        TicketBody::Test(t) => test_tab::test(ui, cx, inputs, &h.key, t),
                        TicketBody::Ship(s) => ship::ship(ui, cx, inputs, s),
                        TicketBody::Timeline(rows) => timeline(ui, rows),
                        TicketBody::Review(_) => div(),
                    })
                    .child(div().h_8()),
            )
            .into_any_element(),
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .text_color(pal.fg)
        .child(head(ui, h, tab))
        .child(content)
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
                .px_4()
                .py_1()
                .border_b_1()
                .border_color(pal.border.opacity(0.6))
                .text_xs()
                .text_color(pal.faint)
                .child(col(60.0).child("Time"))
                .child(col(220.0).child("Action"))
                .child(col(100.0).child("Repo"))
                .child(col(80.0).child("Outcome"))
                .child(div().flex_1().px_2().child("Details")),
        )
        .children(rows.iter().map(|a| {
            div()
                .flex()
                .items_start()
                .px_4()
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
                        .min_w_0()
                        .px_2()
                        .flex()
                        .gap_2()
                        .when(show_ticket, |d| {
                            d.when_some(a.ticket.clone(), |d, k| d.child(key_text(ui, &k)))
                        })
                        .child(div().flex_1().min_w_0().text_xs().child(a.details.clone())),
                )
        }))
}

pub fn audit_rows(ui: &Ui, rows: &[AuditRow], show_ticket: bool) -> Div {
    audit_table(ui, rows, show_ticket)
}

/// The ticket's audit log. The tab already says what it is, so there is no title.
fn timeline(ui: &Ui, rows: &[AuditRow]) -> Div {
    div().child(if rows.is_empty() {
        div()
            .py_8()
            .flex()
            .justify_center()
            .child(empty_panel(
                ui,
                &EmptyVm::new(
                    "No activity yet",
                    "Changes to this ticket's state and anything sent to Jira or GitHub for it are logged here.",
                ),
            ))
            .into_any_element()
    } else {
        audit_table(ui, rows, false).into_any_element()
    })
}
