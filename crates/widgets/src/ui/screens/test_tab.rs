//! Test tab: what activation will do, or the checklist and environment of the active ticket. Full-width bands.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::widgets::*;
use crate::vm::*;

/// Where each repo will be (or is): flush rows, hairlines edge to edge. Only ticket branches are tagged.
fn env_table(ui: &Ui, rows: &[EnvRow]) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(pal.border.opacity(0.4))
        .children(rows.iter().map(|r| {
            div()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py_1p5()
                .border_b_1()
                .border_color(pal.border.opacity(0.4))
                .child(
                    div()
                        .w(px(100.0))
                        .flex_none()
                        .font_family(ui.mono.clone())
                        .text_sm()
                        .child(r.repo.to_string()),
                )
                .child(
                    div()
                        .w(px(110.0))
                        .flex_none()
                        .child(if r.role.tone == Tone::Accent {
                            pill(pal, &r.role).into_any_element()
                        } else {
                            faint(pal, r.role.text.clone()).into_any_element()
                        }),
                )
                .child(
                    div()
                        .font_family(ui.mono.clone())
                        .text_sm()
                        .child(r.branch.to_string()),
                )
                .child(faint(pal, r.note.clone()))
        }))
}

fn overlay_note(ui: &Ui, text: &str) -> Div {
    warn_box(
        &ui.pal,
        Tone::Warn,
        [div().child(format!("⚡ {text}")).into_any_element()],
    )
}

pub fn test(ui: &Ui, _cx: &App, inputs: &Inputs, key: &TicketKey, t: &TestVm) -> Div {
    let pal = &ui.pal;
    match t {
        TestVm::Inactive {
            why,
            errors,
            plan,
            overlay,
            activate,
        } => div()
            .flex()
            .flex_col()
            .child(band(
                pal,
                true,
                "Activation",
                pad(div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(muted(pal, format!("Not active. {why}")))
                    .when(!errors.is_empty(), |d| {
                        d.child(warn_box(
                            pal,
                            Tone::Warn,
                            errors
                                .iter()
                                .map(|e| div().child(e.clone()).into_any_element()),
                        ))
                    })
                    .when_some(overlay.clone(), |d, o| d.child(overlay_note(ui, &o)))
                    .when_some(activate.clone(), |d, a| {
                        d.child(div().flex().child(button(ui, &a)))
                    })),
            ))
            .child(band(pal, false, "If you activate now", env_table(ui, plan))),
        TestVm::Active {
            checklist,
            done,
            env,
            overlay,
            actions,
            ..
        } => {
            let complete = *done == checklist.len() && *done > 0;
            div()
                .flex()
                .flex_col()
                .child(band(
                    pal,
                    true,
                    format!(
                        "Checklist · {done} of {}{}",
                        checklist.len(),
                        if complete { " · complete" } else { "" }
                    ),
                    pad(div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .children(checklist.iter().enumerate().map(|(i, (text, d))| {
                            check_row(
                                ui,
                                "chk",
                                text,
                                *d,
                                Intent::Do(Command::ToggleChecklist {
                                    key: key.clone(),
                                    index: i,
                                }),
                            )
                        }))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .child(inputs.line(&Field::Checklist(key.clone()))),
                                )
                                .child(button(
                                    ui,
                                    &Btn::new("Add", Intent::Submit(Field::Checklist(key.clone()))),
                                )),
                        )),
                ))
                .child(band(
                    pal,
                    false,
                    "Environment",
                    div()
                        .flex()
                        .flex_col()
                        .child(env_table(ui, env))
                        .child(pad(match overlay {
                            Some(o) => overlay_note(ui, o),
                            None => faint(
                                pal,
                                "No overlay needed: the API client is not on a ticket branch.",
                            ),
                        })),
                ))
                .child(band(
                    pal,
                    false,
                    "Notes",
                    pad(inputs.area(&Field::Notes(key.clone()))),
                ))
                .child(band(pal, false, "Next", pad(buttons(ui, actions))))
        }
    }
}
