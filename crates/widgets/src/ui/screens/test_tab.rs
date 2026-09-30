//! Test tab: what activation will do, or the checklist and environment of the active ticket.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::widgets::*;
use crate::vm::*;

fn env_table(ui: &Ui, rows: &[EnvRow]) -> Div {
    let pal = &ui.pal;
    div().flex().flex_col().children(rows.iter().map(|r| {
        div()
            .flex()
            .items_center()
            .gap_3()
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
            .child(div().w(px(110.0)).flex_none().child(pill(pal, &r.role)))
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
    let pal = &ui.pal;
    warn_box(
        pal,
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
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(pal.border)
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Not active."))
            .child(muted(pal, why.clone()))
            .when(!errors.is_empty(), |d| {
                d.child(warn_box(
                    pal,
                    Tone::Warn,
                    errors
                        .iter()
                        .map(|e| div().child(e.clone()).into_any_element()),
                ))
            })
            .child(section(pal, "If you activate now").child(env_table(ui, plan)))
            .when_some(overlay.clone(), |d, o| d.child(overlay_note(ui, &o)))
            .when_some(activate.clone(), |d, a| {
                d.child(div().child(button(ui, &a)))
            }),
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
                .gap_6()
                .items_start()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_5()
                        .flex_1()
                        .min_w_0()
                        .child(
                            section(pal, "Checklist")
                                .child(div().child(pill(
                                    pal,
                                    &Badge::new(
                                        format!("{done} of {}", checklist.len()),
                                        if complete { Tone::Ok } else { Tone::Neutral },
                                    ),
                                )))
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
                                            &Btn::new(
                                                "Add",
                                                Intent::Submit(Field::Checklist(key.clone())),
                                            ),
                                        )),
                                ),
                        )
                        .child(
                            section(pal, "Notes").child(inputs.area(&Field::Notes(key.clone()))),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_5()
                        .w(px(380.0))
                        .flex_none()
                        .child(section(pal, "Environment").child(env_table(ui, env)).child(
                            match overlay {
                                Some(o) => overlay_note(ui, o),
                                None => faint(
                                    pal,
                                    "No overlay needed: the API client is not on a ticket branch.",
                                ),
                            },
                        ))
                        .child(section(pal, "Next").child(buttons(ui, actions))),
                )
        }
    }
}
