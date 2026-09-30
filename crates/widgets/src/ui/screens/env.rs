//! Screens that are not about one ticket: what is on uat, the workspace, the audit log, settings.
//! Edge to edge: strips, columns and row hairlines reach the panel edges; text keeps the page margin.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::ticket::audit_rows;
use crate::ui::ctx::Ui;
use crate::ui::theme::Pal;
use crate::ui::widgets::*;
use crate::vm::*;

/// A strip across the top of a screen: same height and line as the panel headers.
fn strip(pal: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .flex_none()
        .h_10()
        .px_6()
        .border_b_1()
        .border_color(pal.border)
}

/// A flush row: hairline below, edge to edge.
fn row(pal: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_6()
        .py_2()
        .border_b_1()
        .border_color(pal.border.opacity(0.4))
}

pub fn on_uat(ui: &Ui, v: &OnUatVm) -> Div {
    let pal = &ui.pal;
    let columns = div()
        .flex()
        .w_full()
        .children(v.columns.iter().enumerate().map(|(n, c)| {
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .when(n + 1 < v.columns.len(), |d| {
                    d.border_r_1().border_color(pal.border)
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .h_9()
                        .px_4()
                        .border_b_1()
                        .border_color(pal.border)
                        .child(
                            div()
                                .font_family(ui.mono.clone())
                                .text_xs()
                                .child(c.repo.to_string()),
                        )
                        .child(faint(pal, c.host.clone())),
                )
                .child(if c.items.is_empty() {
                    div()
                        .px_4()
                        .py_3()
                        .child(faint(pal, "Nothing merged by you."))
                } else {
                    div().flex().flex_col().children(c.items.iter().map(|i| {
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .px_4()
                            .py_2()
                            .border_b_1()
                            .border_color(pal.border.opacity(0.4))
                            .child(
                                div()
                                    .flex()
                                    .justify_between()
                                    .child(key_link(
                                        ui,
                                        &i.key,
                                        Intent::go_ticket(i.key.clone(), TicketTab::Ship),
                                    ))
                                    .child(
                                        div()
                                            .font_family(ui.mono.clone())
                                            .text_xs()
                                            .text_color(pal.faint)
                                            .child(i.commit.clone()),
                                    ),
                            )
                            .child(div().text_sm().child(i.title.clone()))
                            .child(faint(
                                pal,
                                match &i.chip {
                                    Some(c) if c.tone != Tone::Ok => {
                                        format!("{} · {}", c.text, i.jira.text)
                                    }
                                    _ => format!("deployed · {}", i.jira.text),
                                },
                            ))
                    }))
                })
        }));
    div().flex().flex_col().child(columns).child(band(
        pal,
        false,
        "Potential overlap",
        pad(if v.overlaps.is_empty() {
            faint(pal, "No overlapping files between tickets on uat.")
        } else {
            div().flex().flex_col().gap_2().children(
                v.overlaps.iter().map(|o| {
                    warn_box(pal, Tone::Warn, [div().child(o.clone()).into_any_element()])
                }),
            )
        }),
    ))
}

pub fn workspace(ui: &Ui, v: &WorkspaceVm) -> Div {
    let pal = &ui.pal;
    let col = |w: f32| div().flex_none().w(px(w)).px_2();
    div()
        .flex()
        .flex_col()
        .child(
            strip(pal)
                .child(div().child(v.name.clone()))
                .child(
                    div()
                        .font_family(ui.mono.clone())
                        .text_xs()
                        .text_color(pal.faint)
                        .child(v.order.clone()),
                )
                .child(div().flex_1())
                .child(if v.up {
                    faint(pal, "services up").into_any_element()
                } else {
                    pill(pal, &Badge::new("services down", Tone::Warn)).into_any_element()
                })
                .child(button(ui, &v.toggle)),
        )
        .child(
            div()
                .flex()
                .px_4()
                .py_1()
                .border_b_1()
                .border_color(pal.border.opacity(0.6))
                .text_xs()
                .text_color(pal.faint)
                .child(col(120.0).child("Project"))
                .child(col(80.0).child("Services"))
                .child(col(220.0).child("Branch"))
                .child(div().flex_1().px_2().child("State")),
        )
        .children(v.rows.iter().map(|r| {
            div()
                .flex()
                .items_center()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(pal.border.opacity(0.4))
                .child(
                    col(120.0)
                        .font_family(ui.mono.clone())
                        .text_sm()
                        .child(r.repo.to_string()),
                )
                .child(col(80.0).text_sm().child(r.services.to_string()))
                .child(
                    col(220.0)
                        .font_family(ui.mono.clone())
                        .text_sm()
                        .child(r.branch.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .px_2()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(if r.badges.is_empty() {
                            faint(pal, "clean").into_any_element()
                        } else {
                            pills(pal, &r.badges).into_any_element()
                        })
                        .when_some(r.break_lock.clone(), |d, a| d.child(button(ui, &a))),
                )
        }))
        .when_some(v.note.clone(), |d, n| {
            d.child(div().px_6().py_3().child(faint(pal, n)))
        })
}

pub fn audit(ui: &Ui, rows: &[AuditRow]) -> Div {
    let pal = &ui.pal;
    div().child(if rows.is_empty() {
        pad(empty_state(pal, "Nothing yet. Do something.")).into_any_element()
    } else {
        audit_rows(ui, rows, true).into_any_element()
    })
}

pub fn settings(ui: &Ui, v: &SettingsVm) -> Div {
    let pal = &ui.pal;
    let kv_row = |k: &str, val: String| {
        row(pal)
            .justify_between()
            .child(div().text_sm().text_color(pal.muted).child(k.to_string()))
            .child(div().text_sm().child(val))
    };
    div()
        .flex()
        .flex_col()
        .child(band(
            pal,
            true,
            "Appearance",
            pad(div().flex().gap_1().children(
                v.appearance
                    .iter()
                    .map(|(label, on, intent)| chip(ui, label.clone(), *on, intent.clone())),
            )),
        ))
        .child(band(
            pal,
            false,
            "Providers",
            div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(pal.border.opacity(0.4))
                .children(v.providers.iter().map(|p| {
                    row(pal)
                        .child(div().text_sm().child(p.name.clone()))
                        .child(
                            div()
                                .font_family(ui.mono.clone())
                                .text_xs()
                                .text_color(pal.faint)
                                .child(p.version.clone()),
                        )
                        .child(div().flex_1())
                        .child(if p.ready {
                            faint(pal, "ready").into_any_element()
                        } else {
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(pill(pal, &Badge::new("signed out", Tone::Warn)))
                                .child(faint(pal, format!("run {}", p.login_hint)))
                                .into_any_element()
                        })
                        .child(button(ui, &p.toggle))
                }))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .px_6()
                        .py_3()
                        .child(button(
                            ui,
                            &Btn::new("Re-check", Intent::Do(Command::Recheck)),
                        ))
                        .child(button(ui, &Btn::new("Diagnose…", Intent::Diagnose))),
                ),
        ))
        .child(band(
            pal,
            false,
            "Jira mapping",
            div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(pal.border.opacity(0.4))
                .child(
                    row(pal)
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .text_color(pal.muted)
                                .child("Wait for a PR or resolved comments"),
                        )
                        .child(div().flex().gap_1().children(
                            v.wait_options.iter().map(|(label, on, a)| {
                                chip(ui, label.clone(), *on, a.intent.clone())
                            }),
                        )),
                )
                .children(v.mapping.iter().map(|(k, val)| kv_row(k, val.clone()))),
        ))
        .child(band(
            pal,
            false,
            "Repos",
            div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(pal.border.opacity(0.4))
                .children(v.repos.iter().map(|r| {
                    row(pal)
                        .text_sm()
                        .child(
                            div()
                                .w(px(110.0))
                                .font_family(ui.mono.clone())
                                .child(r[0].clone()),
                        )
                        .child(
                            div()
                                .w(px(90.0))
                                .font_family(ui.mono.clone())
                                .child(r[1].clone()),
                        )
                        .child(
                            div()
                                .w(px(90.0))
                                .font_family(ui.mono.clone())
                                .child(r[2].clone()),
                        )
                        .child(div().text_color(pal.muted).child(r[3].clone()))
                })),
        ))
        .child(band(
            pal,
            false,
            "Data",
            div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(pal.border.opacity(0.4))
                .children(v.data.iter().map(|(k, val)| kv_row(k, val.clone())))
                .child(div().flex().px_6().py_3().child(button(ui, &v.reset))),
        ))
}
