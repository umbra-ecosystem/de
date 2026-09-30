//! Home: the ranked suggestions, each with the reason it exists. A uniform list of fixed-height rows, like Zed's.
//!
//! Clicking a row opens its ticket at the tab that matters; the primary action is the button on hover. Snooze and dismiss are icon buttons
//! that appear on hover; what a suggestion is (hotfix, automatic, sends to a remote) is an icon with a tooltip.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::filter_menu::{FilterGroup, FilterRow, filter_menu};
use crate::ui::widgets::*;
use crate::vm::*;

pub const ROW_H: f32 = 56.0;

/// A small icon that explains itself on hover.
fn icon_tip(
    id: impl Into<ElementId>,
    icon: IconName,
    color: Hsla,
    tip: &'static str,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .child(Icon::new(icon).small().text_color(color))
        .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
}

/// The icon for a suggestion's primary action.
fn quick_icon(intent: &Intent) -> IconName {
    match intent {
        Intent::Do(c) => match c {
            Command::Claim(_) | Command::Reclaim(_) => IconName::Plus,
            Command::StartReview(_) | Command::Activate { .. } | Command::Prepare(_) => {
                IconName::Play
            }
            Command::Sync => IconName::RefreshCw,
            Command::Rerun { .. } => IconName::Redo2,
            Command::Park(_) => IconName::Pause,
            Command::BreakLock(_) => IconName::Delete,
            Command::MarkReviewed(_)
            | Command::Approve { .. }
            | Command::ApproveAll(_)
            | Command::Transition(_) => IconName::CircleCheck,
            Command::ComposeDraft(_) => IconName::FileText,
            _ => IconName::ArrowRight,
        },
        _ => IconName::ArrowRight,
    }
}

fn icon_button(id: impl Into<ElementId>, icon: IconName, tip: &'static str) -> Button {
    Button::new(id).ghost().small().icon(icon).tooltip(tip)
}

/// One suggestion as a list row.
pub fn suggestion_row(ui: &Ui, s: &SuggestionCard) -> Stateful<Div> {
    let pal = &ui.pal;
    let dim = matches!(s.state, SugState::Dismissed | SugState::Snoozed);
    let group = SharedString::from(format!("row-{}", s.id));

    let mut marks: Vec<AnyElement> = Vec::new();
    if s.hotfix {
        marks.push(
            icon_tip(
                SharedString::from(format!("hot-{}", s.id)),
                IconName::TriangleAlert,
                pal.hot,
                "Hotfix",
            )
            .into_any_element(),
        );
    }
    match s.level {
        Level::External => {
            marks.push(
                icon_tip(
                    SharedString::from(format!("ext-{}", s.id)),
                    IconName::ExternalLink,
                    pal.bad,
                    "Sends to a remote, after a confirmation",
                )
                .into_any_element(),
            );
        }
        Level::Automatic => {
            marks.push(
                icon_tip(
                    SharedString::from(format!("auto-{}", s.id)),
                    IconName::Bot,
                    pal.muted,
                    "Automatic",
                )
                .into_any_element(),
            );
        }
        Level::Local => {}
    }
    if s.info {
        marks.push(
            icon_tip(
                SharedString::from(format!("info-{}", s.id)),
                IconName::Info,
                pal.muted,
                "Informational: nothing to send",
            )
            .into_any_element(),
        );
    }

    let title = div()
        .flex()
        .items_center()
        .gap_2()
        .min_w_0()
        .when(!marks.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .flex_none()
                    .children(marks),
            )
        })
        .when_some(s.ticket.clone(), |d, k| d.child(key_text(ui, &k)))
        .child(
            div()
                .truncate()
                .when(s.attention, |d| d.font_weight(FontWeight::BOLD))
                .child(s.title.clone()),
        )
        .when(s.state == SugState::Resurfaced, |d| {
            d.child(dot(pal.accent))
        });

    // Hover actions. Clicks on them must not also click the row.
    let mut actions = div()
        .id(SharedString::from(format!("acts-{}", s.id)))
        .flex()
        .items_center()
        .gap_0p5()
        .flex_none()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    if dim {
        actions = actions.child(
            icon_button(
                SharedString::from(format!("undo-{}", s.id)),
                IconName::Undo2,
                "Bring it back",
            )
            .on_click(ui.on_click(s.undo.intent.clone())),
        );
    } else {
        // The contextual quick action: what clicking the row does, spelled out, on hover.
        actions = actions.child(
            Button::new(SharedString::from(format!("go-{}", s.id)))
                .primary()
                .small()
                .icon(quick_icon(&s.primary.intent))
                .label(s.primary.label.clone())
                .on_click(ui.on_click(s.primary.intent.clone())),
        );
        if let Some(alt) = s.alt.clone() {
            actions = actions.child(
                Button::new(SharedString::from(format!("alt-{}", s.id)))
                    .ghost()
                    .small()
                    .icon(IconName::Check)
                    .tooltip(alt.label.clone())
                    .on_click(ui.on_click(alt.intent.clone())),
            );
        }
        let snoozes: Vec<Btn> = s.snooze.clone();
        let menu_ui = ui.clone();
        actions = actions
            .child(
                icon_button(
                    SharedString::from(format!("snooze-{}", s.id)),
                    IconName::Bell,
                    "Snooze",
                )
                .dropdown_menu(move |menu, _, _| {
                    let mut menu = menu;
                    for a in &snoozes {
                        let ui = menu_ui.clone();
                        let intent = a.intent.clone();
                        menu = menu.item(
                            PopupMenuItem::new(format!("Snooze for {}", a.label))
                                .on_click(move |_, _, cx| ui.send(intent.clone(), cx)),
                        );
                    }
                    menu
                }),
            )
            .child(
                icon_button(
                    SharedString::from(format!("dismiss-{}", s.id)),
                    IconName::Close,
                    "Dismiss",
                )
                .on_click(ui.on_click(s.dismiss.intent.clone())),
            );
    }

    div()
        .id(SharedString::from(format!("suggestion-{}", s.id)))
        .group(group.clone())
        .flex()
        .items_center()
        .gap_3()
        .w_full()
        .h(px(ROW_H))
        .px_4()
        .border_b_1()
        .border_color(pal.border.opacity(0.5))
        .when(dim, |d| d.opacity(0.55))
        // The row opens the ticket; the action is the button that appears on hover. A row about no ticket is inert.
        .when_some(s.open.clone(), |d, open| {
            d.cursor_pointer()
                .hover(|d| d.bg(pal.hover))
                .on_click(ui.on_click(open))
        })
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
        .child(
            div()
                .opacity(if dim { 1.0 } else { 0.0 })
                .group_hover(group, |d| d.opacity(1.0))
                .child(actions),
        )
}

/// A small label above a group of rows, optionally with a coloured dot.
fn section_label(ui: &Ui, text: String, dot_color: Option<Hsla>) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .items_center()
        .gap_2()
        .flex_none()
        .h_8()
        .px_4()
        .text_xs()
        .text_color(pal.muted)
        .when_some(dot_color, |d, c| d.child(dot(c)))
        .child(text)
}

/// The whole screen: a search field and the dismissed/snoozed toggle, then a uniform list that fills the rest.
pub fn next(ui: &Ui, inputs: &Inputs, vm: &NextVm) -> AnyElement {
    let pal = &ui.pal;
    let menu = filter_menu(
        ui,
        "next-filters",
        vec![FilterGroup {
            title: "Show",
            rows: vec![FilterRow::new(
                format!("Dismissed and snoozed ({})", vm.hidden),
                vm.show_all,
                Intent::ToggleShowAll,
            )],
        }],
        None,
    );
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .h(px(48.0))
        .px_4()
        .border_b_1()
        .border_color(pal.border)
        .child(div().flex_1())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .max_w(px(360.0))
                .child(inputs.search(&Field::NextFilter)),
        )
        .child(menu);
    // What needs you is pinned above the rest, so it is the first thing seen and never scrolls away. Dismissed and
    // snoozed rows (shown on request) belong to the rest.
    let (urgent, rest): (Vec<SuggestionCard>, Vec<SuggestionCard>) = vm
        .cards
        .iter()
        .cloned()
        .partition(|c| c.attention && matches!(c.state, SugState::Open | SugState::Resurfaced));
    // A ticket already pinned does not come back below as mere information (a mention of it, say): opening the
    // pinned row shows that. Anything it could act on still appears.
    let rest: Vec<SuggestionCard> = rest
        .into_iter()
        .filter(|c| {
            !(c.info
                && c.ticket
                    .as_ref()
                    .is_some_and(|t| urgent.iter().any(|u| u.ticket.as_ref() == Some(t))))
        })
        .collect();
    let pinned = (!urgent.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .flex_none()
            .bg(pal.bad.opacity(0.06))
            .border_b_1()
            .border_color(pal.border)
            .child(section_label(
                ui,
                format!("Needs attention · {}", urgent.len()),
                Some(pal.bad),
            ))
            .child(
                div()
                    .id("attention-rows")
                    .flex()
                    .flex_col()
                    .max_h(px(ROW_H * 4.0))
                    .overflow_y_scroll()
                    .children(urgent.iter().map(|c| suggestion_row(ui, c))),
            )
    });
    let body = if rest.is_empty() && pinned.is_none() {
        div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .child(empty_panel(ui, &vm.empty))
            .into_any_element()
    } else if rest.is_empty() {
        div().flex_1().into_any_element()
    } else {
        let cards = Rc::new(rest);
        let list_ui = ui.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .when(pinned.is_some(), |d| {
                d.child(section_label(ui, "Up next".to_string(), None))
            })
            .child(
                uniform_list("next-list", cards.len(), move |range, _, _| {
                    range
                        .map(|i| suggestion_row(&list_ui, &cards[i]))
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
        .child(header)
        .children(pinned)
        .child(body)
        .into_any_element()
}
