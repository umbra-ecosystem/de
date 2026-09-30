//! Next: the ranked suggestions, each with the reason it exists. A uniform list of fixed-height rows, like Zed's.

use std::rc::Rc;

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::Ui;
use crate::ui::widgets::*;
use crate::vm::*;

pub const ROW_H: f32 = 64.0;

/// One suggestion as a list row: rank, badges, key, title and reason on the left, its actions on the right.
pub fn suggestion_row(ui: &Ui, s: &SuggestionCard) -> Div {
    let pal = &ui.pal;
    let dim = matches!(s.state, SugState::Dismissed | SugState::Snoozed);
    let level = match s.level {
        Level::External => Some(Badge::new("sends to a remote", Tone::Bad)),
        Level::Automatic => Some(Badge::new("automatic", Tone::Neutral)),
        Level::Local => None,
    };
    let title = div()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .when(s.hotfix, |d| d.child(hotfix_pill(pal)))
        .when_some(s.ticket.clone(), |d, k| {
            d.child(key_link(
                ui,
                &k,
                Intent::go_ticket(k.clone(), TicketTab::Overview),
            ))
        })
        .child(
            div()
                .truncate()
                .font_weight(FontWeight::SEMIBOLD)
                .child(s.title.clone()),
        )
        .when(s.info, |d| d.child(faint(pal, "info")))
        .when(s.state == SugState::Resurfaced, |d| {
            d.child(dot(pal.accent))
                .child(faint(pal, "back: the facts changed"))
        });
    let actions = if dim {
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
            .when_some(level, |d, b| d.child(pill(pal, &b)))
            .child(button(ui, &s.primary))
            .when_some(s.alt.clone(), |d, a| d.child(button(ui, &a)))
            .children(s.snooze.iter().map(|a| button_ghost(ui, a)))
            .child(button_ghost(ui, &s.dismiss))
    };
    div()
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .h(px(ROW_H))
        .px_3()
        .border_b_1()
        .border_color(pal.border.opacity(0.5))
        .hover(|d| d.bg(pal.hover))
        .when(dim, |d| d.opacity(0.55))
        .child(
            div()
                .flex_none()
                .w_5()
                .text_color(pal.faint)
                .child(s.rank.to_string()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(title)
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(pal.muted)
                        .child(s.reason.clone()),
                ),
        )
        .child(div().flex_none().child(actions))
}

/// The whole screen: a header and a uniform list that fills the rest.
pub fn next(ui: &Ui, vm: &NextVm) -> AnyElement {
    let pal = &ui.pal;
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .pb_3()
        .child(heading("Next"))
        .child(check_row(
            ui,
            "show-all",
            &format!("show dismissed and snoozed ({})", vm.hidden),
            vm.show_all,
            Intent::ToggleShowAll,
        ));
    let body = if vm.cards.is_empty() {
        empty_state(pal, "Nothing to do.").into_any_element()
    } else {
        let cards = Rc::new(vm.cards.clone());
        let ui = ui.clone();
        uniform_list("next-list", cards.len(), move |range, _, _| {
            range
                .map(|i| suggestion_row(&ui, &cards[i]))
                .collect::<Vec<_>>()
        })
        .w_full()
        .flex_1()
        .into_any_element()
    };
    div()
        .flex()
        .flex_col()
        .size_full()
        .child(header)
        .child(body)
        .into_any_element()
}
