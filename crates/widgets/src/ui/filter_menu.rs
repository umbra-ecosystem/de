//! The filter dropdown: one button, a count of what is narrowing the view, and a popover of checkable rows in
//! titled groups. Adapted from the save panel's view-filter menu. It is a popover rather than a `PopupMenu`
//! because a menu closes on every pick, and several filters are usually changed in one go.
//!
//! A screen supplies the groups and what each row sends; nothing here knows about tickets.

use gpui_kit::component::badge::Badge as CountBadge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Selectable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::ctx::Ui;
use crate::vm::Intent;

const MENU_WIDTH: f32 = 260.0;
const ROW_HEIGHT: f32 = 26.0;

/// One checkable row. A disabled row is shown but cannot be picked (nothing for it to act on).
#[derive(Clone, Debug, PartialEq)]
pub struct FilterRow {
    pub label: String,
    pub checked: bool,
    pub enabled: bool,
    pub intent: Intent,
}

impl FilterRow {
    pub fn new(label: impl Into<String>, checked: bool, intent: Intent) -> Self {
        Self {
            label: label.into(),
            checked,
            enabled: true,
            intent,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilterGroup {
    pub title: &'static str,
    pub rows: Vec<FilterRow>,
}

/// The button and its popover. `clear` adds a "Show everything" row, enabled while anything is on.
pub fn filter_menu(
    ui: &Ui,
    id: &'static str,
    groups: Vec<FilterGroup>,
    clear: Option<Intent>,
) -> impl IntoElement {
    let active = groups
        .iter()
        .flat_map(|g| &g.rows)
        .filter(|r| r.checked)
        .count();
    // Highlighted, with a count, while anything narrows the view, so a hidden row is never a mystery.
    let trigger = Button::new(id)
        .ghost()
        .small()
        .icon(Icon::new(gpui_kit::assets::IconName::Funnel))
        .selected(active > 0)
        .tooltip(if active > 0 {
            format!("Filters ({active} active)")
        } else {
            "Filters".to_string()
        });
    let badge = ui.pal.accent;
    let ui = ui.clone();
    let popover = Popover::new((id, 1usize))
        .anchor(Anchor::TopRight)
        .trigger(trigger)
        .p_0()
        .content(move |_state, _window, cx| {
            let mut menu = div().flex().flex_col().w(px(MENU_WIDTH)).p_1().gap_y_0p5();
            for (g, group) in groups.iter().enumerate() {
                if g > 0 {
                    menu = menu.child(separator(cx));
                }
                menu = menu.child(heading(group.title, cx));
                for (n, row) in group.rows.iter().enumerate() {
                    let ui = ui.clone();
                    let intent = row.intent.clone();
                    menu = menu.child(menu_row(
                        (id, g * 100 + n + 10),
                        row.label.clone().into(),
                        row.checked,
                        row.enabled,
                        cx,
                        move |_window, cx| ui.send(intent.clone(), cx),
                    ));
                }
            }
            if let Some(clear) = clear.clone() {
                let ui = ui.clone();
                menu = menu.child(separator(cx)).child(menu_row(
                    (id, 9),
                    "Show everything".into(),
                    false,
                    active > 0,
                    cx,
                    move |_window, cx| ui.send(clear.clone(), cx),
                ));
            }
            menu
        });
    CountBadge::new().count(active).color(badge).child(popover)
}

/// The look of gpui-component's `PopupMenu` items, which a popover can hold without closing on a click.
/// Every row reserves the check column so labels line up whether or not they are on.
fn menu_row(
    id: (&'static str, usize),
    label: SharedString,
    checked: bool,
    enabled: bool,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let theme = cx.theme();
    let hover_background = theme.tokens.accent;
    let hover_foreground = theme.accent_foreground;
    div()
        .flex()
        .items_center()
        .id(id)
        .h(px(ROW_HEIGHT))
        .px(px(8.))
        .gap_x_1()
        .items_center()
        .justify_between()
        .rounded(theme.radius.min(px(8.)))
        .text_sm()
        .text_color(theme.foreground)
        .child(
            div()
                .flex()
                .items_center()
                .items_center()
                .gap_x_1()
                .child(if checked {
                    Icon::new(IconName::Check).xsmall()
                } else {
                    Icon::empty().xsmall()
                })
                .child(label),
        )
        .when(!enabled, |this| this.text_color(theme.muted_foreground))
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(hover_background).text_color(hover_foreground))
                .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                    cx.stop_propagation();
                })
                .on_click(move |_event, window, cx| on_click(window, cx))
        })
}

fn heading(title: &'static str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .flex()
        .items_center()
        .h(px(ROW_HEIGHT))
        .px(px(8.))
        .gap_x_1()
        .items_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(Icon::empty().xsmall())
        .child(title)
}

fn separator(cx: &App) -> impl IntoElement {
    div()
        .my_0p5()
        .mx_neg_1()
        .border_b(px(1.))
        .border_color(cx.theme().border)
}
