//! Next: the ranked suggestions, each with the reason it exists.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::Ui;
use crate::ui::widgets::*;
use crate::vm::*;

pub fn suggestion_card(ui: &Ui, s: &SuggestionCard) -> Div {
    let pal = &ui.pal;
    let muted_state = matches!(s.state, SugState::Dismissed | SugState::Snoozed);
    let level = match s.level {
        Level::External => Badge::new("sends to a remote", Tone::Bad),
        Level::Automatic => Badge::new("automatic", Tone::Neutral),
        Level::Local => Badge::new("local", Tone::Neutral),
    };
    let body = div()
        .flex()
        .flex_col()
        .gap_1()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .when(s.hotfix, |d| d.child(hotfix_pill(pal)))
                .when(s.info, |d| {
                    d.child(pill(pal, &Badge::new("info", Tone::Neutral)))
                })
                .when_some(s.ticket.clone(), |d, k| {
                    d.child(key_link(ui, &k, Intent::go_ticket(&k, TicketTab::Overview)))
                })
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(s.title.clone()),
                )
                .when(s.state == SugState::Resurfaced, |d| {
                    d.child(dot(pal.accent))
                        .child(faint(pal, "came back because the facts changed"))
                }),
        )
        .child(muted(pal, s.reason.clone()))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .pt_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(button(ui, &s.primary))
                        .when_some(s.alt.clone(), |d, a| d.child(button(ui, &a)))
                        .child(pill(pal, &level)),
                )
                .child(if muted_state {
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(faint(
                            pal,
                            if s.state == SugState::Dismissed {
                                "dismissed"
                            } else {
                                "snoozed"
                            },
                        ))
                        .child(button_ghost(ui, &s.undo))
                } else {
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(faint(pal, "Snooze"))
                        .children(s.snooze.iter().map(|a| button_ghost(ui, a)))
                        .child(button_ghost(ui, &s.dismiss))
                }),
        );
    card(pal)
        .flex_row()
        .gap_3()
        .when(muted_state, |d| d.opacity(0.55))
        .child(
            div()
                .flex_none()
                .w_6()
                .text_color(pal.faint)
                .font_weight(FontWeight::SEMIBOLD)
                .child(s.rank.to_string()),
        )
        .child(body)
}

pub fn next(ui: &Ui, vm: &NextVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(heading("Next"))
                .child(check_row(
                    ui,
                    "show-all",
                    &format!("show dismissed and snoozed ({})", vm.hidden),
                    vm.show_all,
                    Intent::ToggleShowAll,
                )),
        )
        .child(if vm.cards.is_empty() {
            empty_state(pal, "Nothing to do.")
        } else {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .children(vm.cards.iter().map(|c| suggestion_card(ui, c)))
        })
}
