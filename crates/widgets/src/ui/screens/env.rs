//! Screens that are not about one ticket: what is on uat, the workspace, the audit log, settings.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::ticket::audit_rows;
use crate::ui::ctx::Ui;
use crate::ui::widgets::*;
use crate::vm::*;

pub fn on_uat(ui: &Ui, v: &OnUatVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(heading("On uat"))
        .child(
            div()
                .flex()
                .gap_3()
                .items_start()
                .children(v.columns.iter().map(|c| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .font_family(ui.mono.clone())
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(c.repo.to_string()),
                                )
                                .child(pill(pal, &Badge::new(c.host.clone(), Tone::Neutral))),
                        )
                        .child(if c.items.is_empty() {
                            muted(pal, "Nothing merged by you.")
                        } else {
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .children(c.items.iter().map(|i| {
                                    card(pal)
                                        .child(
                                            div()
                                                .flex()
                                                .justify_between()
                                                .child(key_link(
                                                    ui,
                                                    &i.key,
                                                    Intent::go_ticket(&i.key, TicketTab::Ship),
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
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .when_some(i.chip.clone(), |d, c| {
                                                    d.child(deploy_chip(pal, &c))
                                                })
                                                .child(pill(pal, &i.jira)),
                                        )
                                }))
                        })
                })),
        )
        .child(
            section(pal, "Potential overlap").child(if v.overlaps.is_empty() {
                muted(pal, "No overlapping files between tickets on uat.")
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(v.overlaps.iter().map(|o| {
                        warn_box(pal, Tone::Warn, [div().child(o.clone()).into_any_element()])
                    }))
            }),
        )
}

pub fn workspace(ui: &Ui, v: &WorkspaceVm) -> Div {
    let pal = &ui.pal;
    let col = |w: f32| div().flex_none().w(px(w)).px_2();
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(heading(v.name.clone()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(pill(
                            pal,
                            &if v.up {
                                Badge::new("● services up", Tone::Ok)
                            } else {
                                Badge::new("○ services down", Tone::Neutral)
                            },
                        ))
                        .child(button(ui, &v.toggle)),
                ),
        )
        .child(
            div()
                .font_family(ui.mono.clone())
                .text_sm()
                .text_color(pal.muted)
                .child(v.order.clone()),
        )
        .child(
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
                        .child(col(120.0).child("PROJECT"))
                        .child(col(80.0).child("SERVICES"))
                        .child(col(220.0).child("BRANCH"))
                        .child(div().flex_1().px_2().child("STATE")),
                )
                .children(v.rows.iter().map(|r| {
                    div()
                        .flex()
                        .items_center()
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
                                .child(pills(pal, &r.badges))
                                .when_some(r.break_lock.clone(), |d, a| d.child(button(ui, &a))),
                        )
                })),
        )
        .when_some(v.note.clone(), |d, n| {
            d.child(card(pal).child(div().text_sm().child(n)))
        })
}

pub fn audit(ui: &Ui, rows: &[AuditRow]) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(heading("Audit log"))
        .child(if rows.is_empty() {
            empty_state(pal, "Nothing yet. Do something.")
        } else {
            audit_rows(ui, rows, true)
        })
}

pub fn settings(ui: &Ui, v: &SettingsVm) -> Div {
    let pal = &ui.pal;
    let providers = section(pal, "Providers")
        .children(v.providers.iter().map(|p| {
            card(pal)
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(p.name.clone()),
                        )
                        .child(pill(
                            pal,
                            &if p.ready {
                                Badge::new("ready", Tone::Ok)
                            } else {
                                Badge::new("signed out", Tone::Warn)
                            },
                        )),
                )
                .child(
                    div()
                        .font_family(ui.mono.clone())
                        .text_xs()
                        .text_color(pal.muted)
                        .child(p.version.clone()),
                )
                .when(!p.ready, |d| {
                    d.child(div().text_sm().child(format!(
                        "Run {} in a terminal, then re-check.",
                        p.login_hint
                    )))
                })
                .child(div().child(button(ui, &p.toggle)))
        }))
        .child(
            div()
                .flex()
                .gap_2()
                .child(button(
                    ui,
                    &Btn::new("Re-check", Intent::Do(Command::Recheck)),
                ))
                .child(button(ui, &Btn::new("Diagnose…", Intent::Diagnose))),
        );
    let mapping = section(pal, "Jira mapping")
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(pal.muted)
                        .child("Wait for a PR or resolved comments"),
                )
                .child(
                    div().flex().gap_2().children(
                        v.wait_options
                            .iter()
                            .map(|(label, on, a)| chip(ui, label.clone(), *on, a.intent.clone())),
                    ),
                ),
        )
        .children(v.mapping.iter().map(|(k, val)| {
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(faint(pal, k.clone()))
                .child(
                    div()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(pal.border)
                        .text_sm()
                        .child(val.clone()),
                )
        }));
    let repos = section(pal, "Repos")
        .child(div().flex().flex_col().children(v.repos.iter().map(|r| {
            div()
                .flex()
                .gap_3()
                .py_1()
                .border_b_1()
                .border_color(pal.border.opacity(0.4))
                .text_sm()
                .child(
                    div()
                        .w(px(90.0))
                        .font_family(ui.mono.clone())
                        .child(r[0].clone()),
                )
                .child(
                    div()
                        .w(px(70.0))
                        .font_family(ui.mono.clone())
                        .child(r[1].clone()),
                )
                .child(
                    div()
                        .w(px(70.0))
                        .font_family(ui.mono.clone())
                        .child(r[2].clone()),
                )
                .child(div().text_color(pal.muted).child(r[3].clone()))
        })))
        .child(
            section(pal, "Data").children(
                v.data
                    .iter()
                    .map(|(k, val)| kv(pal, &Kv::text(k, val.clone()))),
            ),
        )
        .child(div().child(button(ui, &v.reset)));
    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(heading("Settings"))
        .child(
            div()
                .flex()
                .gap_6()
                .items_start()
                .child(div().flex_1().child(providers))
                .child(div().flex_1().child(mapping))
                .child(div().flex_1().child(repos)),
        )
}
