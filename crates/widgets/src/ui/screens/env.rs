//! Screens that are not about one ticket: what is on uat, the workspace, the audit log, settings.
//! Edge to edge: strips, columns and row hairlines reach the panel edges; text keeps the page margin.

use gpui_kit::prelude::*;
use gpui_kit::*;

use super::ticket::audit_rows;
use crate::ui::ctx::{Inputs, Ui};
use crate::ui::theme::{Pal, mono};
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
        .px_4()
        .border_b_1()
        .border_color(pal.border)
}

/// A flush row: hairline below, edge to edge.
/// Height of a row of the settings page, so a row of chips and a row with an editable value line up.
const SETTING_ROW_H: f32 = 40.0;

fn row(pal: &Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .py_2()
        .border_b_1()
        .border_color(pal.border.opacity(0.4))
}

/// What is on uat: the ticket table every list uses, then the overlaps. Overlaps are exceptions, so there is a
/// band for them only when there are some.
pub fn on_uat(ui: &Ui, inputs: &Inputs, v: &OnUatVm) -> AnyElement {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .child(super::tickets::tickets(ui, inputs, &v.table)),
        )
        .when(!v.overlaps.is_empty(), |d| {
            d.child(band(
                pal,
                false,
                "Potential overlap",
                pad(div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(v.overlaps.iter().map(|o| {
                        warn_box(pal, Tone::Warn, [div().child(o.clone()).into_any_element()])
                    }))),
            ))
        })
        .into_any_element()
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
            d.child(div().px_4().py_3().child(faint(pal, n)))
        })
}

pub fn audit(ui: &Ui, rows: &[AuditRow]) -> Div {
    div().child(if rows.is_empty() {
        div()
            .py_8()
            .flex()
            .justify_center()
            .child(empty_panel(
                ui,
                &EmptyVm::new(
                    "Nothing logged yet",
                    "Every remote write and every change to a ticket's state is recorded here, with its outcome.",
                ),
            ))
            .into_any_element()
    } else {
        audit_rows(ui, rows, true).into_any_element()
    })
}

/// The raw sync logs: the runs on the left, the selected one on the right. A log can be large, so its lines are a
/// `uniform_list` of fixed-height rows (long lines are cut at the edge; the file on disk has them whole).
pub fn logs(ui: &Ui, cx: &App, v: &LogsVm) -> Div {
    let pal = &ui.pal;
    if v.runs.is_empty() {
        return div().size_full().py_8().flex().justify_center().child(empty_panel(
            ui,
            &EmptyVm::new(
                "No sync logs yet",
                "Every sync writes a raw log of what it asked Jira for and what came back. Run a sync and it appears here.",
            )
            .action(Btn::new("Sync now", Intent::Do(Command::Sync)).primary()),
        ));
    }
    let list = div()
        .flex()
        .flex_col()
        .flex_none()
        .w(px(280.0))
        .h_full()
        .border_r_1()
        .border_color(pal.border.opacity(0.6))
        .child(div().px_4().py_3().child(faint(pal, v.note.clone())))
        .child(
            div()
                .id("log-runs")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .children(v.runs.iter().map(|r| {
                    let on = v.selected.as_deref() == Some(r.id.as_str());
                    div()
                        .id(hash_id("log-run", &r.id))
                        .flex()
                        .items_center()
                        .justify_between()
                        .h_9()
                        .px_4()
                        .text_sm()
                        .cursor_pointer()
                        .when(on, |d| d.bg(pal.border.opacity(0.35)))
                        .hover(|d| d.bg(pal.border.opacity(0.2)))
                        .on_click(ui.on_click(Intent::SelectLog(r.id.clone())))
                        .child(r.when.clone())
                        .child(faint(pal, r.size.clone()))
                })),
        );

    let lines = v.lines.clone();
    let font = mono(cx);
    let viewer = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h_10()
                .px_4()
                .border_b_1()
                .border_color(pal.border.opacity(0.6))
                .child(faint(
                    pal,
                    match &v.selected {
                        Some(_) => format!("{} lines", v.lines.len()),
                        None => "Pick a run".to_string(),
                    },
                ))
                .children(v.selected.iter().map(|id| {
                    button_ghost(ui, &Btn::new("Reload", Intent::SelectLog(id.clone())))
                })),
        )
        .child(if v.selected.is_some() {
            uniform_list("log-lines", lines.len(), move |range, _, _| {
                range
                    .map(|i| {
                        div()
                            .h_5()
                            .px_4()
                            .flex()
                            .items_center()
                            .font_family(font.clone())
                            .text_xs()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(lines[i].clone())
                            .into_any_element()
                    })
                    .collect::<Vec<_>>()
            })
            .w_full()
            .flex_1()
            .into_any_element()
        } else {
            div().into_any_element()
        });
    div().flex().size_full().child(list).child(viewer)
}

pub fn settings(ui: &Ui, inputs: &Inputs, v: &SettingsVm) -> Div {
    let pal = &ui.pal;
    let kv_row = |k: &str, val: String| {
        row(pal)
            .justify_between()
            .child(
                div()
                    .flex_none()
                    .text_sm()
                    .text_color(pal.muted)
                    .child(k.to_string()),
            )
            .child(div().flex_1().min_w_0().text_sm().text_right().child(val))
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
                        .px_4()
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
            "Sync",
            div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(pal.border.opacity(0.4))
                .child(
                    row(pal)
                        .h(px(SETTING_ROW_H))
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .text_color(pal.muted)
                                .child("Sync automatically every"),
                        )
                        .child(div().flex().gap_1().children(
                            v.sync_options.iter().map(|(label, on, a)| {
                                chip(ui, label.clone(), *on, a.intent.clone())
                            }),
                        )),
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
                        .h(px(SETTING_ROW_H))
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
                .children(v.mapping.iter().map(|r| {
                    row(pal)
                        .h(px(SETTING_ROW_H))
                        .justify_between()
                        .child(
                            div()
                                .flex_none()
                                .w(px(200.0))
                                .text_sm()
                                .text_color(pal.muted)
                                .child(r.key.label()),
                        )
                        .child(div().flex_1().min_w_0().child(inputs.flat(&Field::Mapping(r.key))))
                        // Something unsaved is an exception, so only then is the row tinted.
                        .when(v.edited.contains(&r.key), |d| d.bg(pal.accent.opacity(0.12)))
                }))
                .child({
                    let none = v.edited.is_empty();
                    let nothing = || Some("No unsaved changes.".to_string());
                    row(pal)
                        .h(px(SETTING_ROW_H))
                        .justify_end()
                        // The way out on the left, the way forward (primary) on the right.
                        .child(button_ghost(
                            ui,
                            &Btn::new("Reset to defaults…", Intent::Do(Command::ResetMapping)),
                        ))
                        .child(button(
                            ui,
                            &Btn::new("Discard", Intent::DiscardMapping)
                                .disabled(if none { nothing() } else { None }),
                        ))
                        .child(button(
                            ui,
                            &Btn::new("Save", Intent::SaveMapping)
                                .primary()
                                .disabled(if none { nothing() } else { None }),
                        ))
                }),
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
                .child(div().flex().px_4().py_3().child(button(ui, &v.reset))),
        ))
}
