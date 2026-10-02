//! The window chrome: toolbar, navigation, tab strip, right panel, status bar, and the overlays above everything.

use gpui_kit::component::badge::Badge as CountBadge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Icon, IconName, Selectable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::ctx::{Inputs, Ui};
use super::screens::next::suggestion_row;
use super::widgets::*;
use crate::vm::*;

/* ------------------------------ title bar ------------------------------ */

/// What lives in the title bar, next to the window controls: the app name, jump-to, and the window-level toggles.
/// Put inside a [`TitleBar`](gpui_kit::component::TitleBar).
pub fn title_bar_content(ui: &Ui, vm: &AppVm, simulate_on: bool) -> Div {
    let pal = &ui.pal;
    let n = vm.attention_count;
    let alert = n > 0;

    // Needs attention: a quiet bell until something needs you, then a red alert with the count that pulses.
    let attention = Button::new("attn")
        .ghost()
        .small()
        .icon(if alert {
            IconName::TriangleAlert
        } else {
            IconName::Bell
        })
        .text_color(if alert { pal.bad } else { pal.muted })
        .selected(vm.attention.is_some())
        .tooltip(if alert {
            format!("Needs attention ({n})")
        } else {
            "Nothing needs you".to_string()
        })
        .on_click(ui.on_click(Intent::ToggleAttention));

    let attention = if alert {
        div()
            .child(CountBadge::new().count(n).color(pal.bad).child(attention))
            .with_animation(
                "attention-pulse",
                Animation::new(std::time::Duration::from_millis(1800))
                    .repeat()
                    .with_easing(pulsating_between(0.55, 1.0)),
                |d, v| d.opacity(v),
            )
            .into_any_element()
    } else {
        div()
            .child(CountBadge::new().count(0).child(attention))
            .into_any_element()
    };

    // The details dock: the icon shows what clicking does, and it never looks "selected".
    let (panel_icon, panel_tip) = if vm.right_open {
        (IconName::PanelRightClose, "Hide details")
    } else {
        (IconName::PanelRightOpen, "Show details")
    };

    let panel = Button::new("panel")
        .ghost()
        .small()
        .icon(panel_icon)
        .tooltip(panel_tip)
        .on_click(ui.on_click(Intent::TogglePanel));

    let simulate = Button::new("sim")
        .ghost()
        .small()
        .icon(IconName::Globe)
        .tooltip("Simulate the outside world")
        .selected(simulate_on)
        .on_click(ui.on_click(Intent::ToggleSimulate));

    div()
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .pr_3()
        .child(workspace_button(ui, vm))
        .child(simulate)
        .child(div().flex_1())
        .child(attention)
        .child(panel)
}

/// The title bar of the page before the dock: just the app name beside the window controls.
pub fn title_bar_minimal() -> Div {
    div()
        .flex()
        .items_center()
        .w_full()
        .child(div().child("de"))
}

/// The title bar's workspace button: the open workspace's name (or a prompt when there is none) and a chevron.
fn workspace_button(ui: &Ui, vm: &AppVm) -> Button {
    let pal = &ui.pal;
    Button::new("workspace")
        .ghost()
        .small()
        .label(vm.workspace.label.clone())
        .text_color(if vm.workspace.none { pal.warn } else { pal.fg })
        .selected(vm.picker.is_some())
        .tooltip("Switch workspace")
        .on_click(ui.on_click(Intent::ToggleWorkspacePicker))
}

/// One workspace in the menu: its name, what it holds, when it was last used and a check when it is the open one.
fn workspace_row(ui: &Ui, w: &WorkspaceItemVm) -> Stateful<Div> {
    let pal = &ui.pal;
    div()
        .id(hash_id("ws-pick", &w.name))
        .flex()
        .items_center()
        .gap_2()
        .h_8()
        .px_3()
        .rounded_md()
        .cursor_pointer()
        .hover(|d| d.bg(pal.hover))
        .on_click(ui.on_click(Intent::Do(Command::SelectWorkspace(w.name.clone()))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .child(w.name.to_string()),
        )
        .when(w.up, |d| d.child(dot(pal.ok)))
        .child(faint(pal, w.last_used.clone()))
        .when(w.current, |d| {
            d.child(Icon::new(IconName::Check).small().text_color(pal.accent))
        })
}

/// A workspace in the modal that lists them all: its name and its projects on one line, and when it was last used.
fn all_workspaces_row(ui: &Ui, w: &WorkspaceItemVm) -> Stateful<Div> {
    let pal = &ui.pal;

    div()
        .id(hash_id("all-ws", &w.name))
        .flex()
        .items_center()
        .gap_3()
        .h_9()
        .px_2()
        .rounded_md()
        .cursor_pointer()
        .hover(|d| d.bg(pal.hover))
        .on_click(ui.on_click(Intent::Do(Command::SelectWorkspace(w.name.clone()))))
        .child(
            div()
                .flex_none()
                .w(px(120.0))
                .truncate()
                .text_sm()
                .child(w.name.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(pal.muted)
                .child(w.projects.join(" \u{b7} ")),
        )
        .when(w.up, |d| d.child(dot(pal.ok)))
        .child(faint(pal, w.last_used.clone()))
        .when(w.current, |d| {
            d.child(Icon::new(IconName::Check).small().text_color(pal.accent))
        })
}

/// One step of a sequence: a mark for where it is, what it does, and its result on the right.
fn step_row(ui: &Ui, st: &StepVm) -> Div {
    let pal = &ui.pal;
    let mark = match st.state {
        ProgressState::Waiting => div()
            .size_1p5()
            .rounded_full()
            .bg(pal.faint.opacity(0.6))
            .into_any_element(),
        ProgressState::Running => gpui_kit::component::spinner::Spinner::new()
            .xsmall()
            .color(pal.accent)
            .into_any_element(),
        ProgressState::Done => Icon::new(IconName::Check)
            .small()
            .text_color(pal.ok)
            .into_any_element(),
        ProgressState::Failed => Icon::new(IconName::TriangleAlert)
            .small()
            .text_color(pal.bad)
            .into_any_element(),
        ProgressState::Skipped => div()
            .text_color(pal.faint)
            .child("\u{2013}")
            .into_any_element(),
    };

    div()
        .flex()
        // The mark and the label sit on the result's first line: a result that wraps must not
        // push them down into the middle of it.
        .items_start()
        .gap_3()
        .min_h_6()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_none()
                        .w(px(16.0))
                        .flex()
                        .justify_center()
                        .child(mark),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(match st.state {
                            ProgressState::Waiting | ProgressState::Skipped => pal.faint,
                            _ => pal.fg,
                        })
                        .child(st.label.clone()),
                ),
        )
        .when(!st.detail.is_empty(), |d| {
            d.child(
                div()
                    .flex_none()
                    .max_w(px(260.0))
                    .text_xs()
                    .text_right()
                    .text_color(if st.state == ProgressState::Failed {
                        pal.bad
                    } else {
                        pal.muted
                    })
                    .child(st.detail.clone()),
            )
        })
}

/// The modal that opens or closes a workspace: what is being done, step by step. Nothing can close it while it
/// runs; when it has stopped it offers what is left to decide.
pub fn sequence_modal(ui: &Ui, q: &SequenceVm) -> Div {
    let pal = &ui.pal;
    let running = q.state == SequenceState::Running;
    let several = q.phases.len() > 1;
    let footer = match q.state {
        SequenceState::Running => div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .child(faint(pal, "This cannot be closed while it runs."))
            .child(faint(pal, q.progress.clone())),
        SequenceState::Ready => div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .child(div().text_sm().text_color(pal.ok).child("Ready"))
            .child(faint(pal, q.progress.clone())),
        SequenceState::NeedsDecision => div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .gap_3()
            .child(div().flex_1().min_w_0().child(faint(
                pal,
                "Something went wrong. You decide what happens next.",
            )))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_2()
                    .children(q.actions.iter().map(|a| button(ui, a))),
            ),
    };

    sheet_frame_with(
        ui,
        520.0,
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().child(q.title.clone()))
            .when(running, |d| d.child(faint(pal, "\u{2026}"))),
        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(q.phases.iter().map(|ph| {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(several, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(pal.muted)
                                .child(ph.title.clone()),
                        )
                    })
                    .children(ph.steps.iter().map(|s| step_row(ui, s)))
            })),
        footer,
        None,
    )
}

/// The workspace menu, hanging from the title bar: search, the open workspace, the others by recency, and how to
/// make a new one.
pub fn workspace_picker(ui: &Ui, inputs: &Inputs, p: &PickerVm) -> Div {
    let pal = &ui.pal;
    let label = |text: &str| {
        div()
            .px_3()
            .pt_2()
            .pb_1()
            .text_xs()
            .text_color(pal.muted)
            .child(text.to_string())
    };

    let none_match = p.current.is_none() && p.recent.is_empty();

    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(scrim(ui, Intent::ToggleWorkspacePicker, false))
        .child(
            div()
                .absolute()
                .top(px(40.0))
                .left(px(12.0))
                .w(px(380.0))
                .max_h(px(560.0))
                .occlude()
                .flex()
                .flex_col()
                .rounded_lg()
                .border_1()
                .border_color(pal.border)
                .bg(pal.bg)
                .shadow_lg()
                .child(
                    div()
                        .p_2()
                        .border_b_1()
                        .border_color(pal.border.opacity(0.6))
                        .child(inputs.line(&Field::WorkspaceSearch)),
                )
                .child(
                    div()
                        .id("ws-list")
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .p_1()
                        .when_some(p.current.clone(), |d, w| {
                            d.child(label("This window")).child(workspace_row(ui, &w))
                        })
                        .when(!p.recent.is_empty(), |d| {
                            d.child(label("Recent workspaces"))
                                .children(p.recent.iter().map(|w| workspace_row(ui, w)))
                        })
                        .when(none_match, |d| {
                            d.child(div().px_3().py_3().child(muted(
                                pal,
                                if p.total == 0 {
                                    "No workspaces yet.".to_string()
                                } else {
                                    format!("Nothing called \u{201c}{}\u{201d}.", p.query.trim())
                                },
                            )))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(pal.border.opacity(0.6))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(pal.muted)
                                .child("Create one with")
                                .child(
                                    div()
                                        .font_family(ui.mono.clone())
                                        .text_color(pal.fg)
                                        .child(p.init_command.clone()),
                                ),
                        )
                        .when(p.current.is_some(), |d| {
                            d.child(button_ghost(
                                ui,
                                &Btn::new("Close workspace", Intent::Do(Command::CloseWorkspace)),
                            ))
                        }),
                ),
        )
}

/* ------------------------------ navigation ------------------------------ */

/// The title strip at the top of a side panel. Same height, border and type as the tab strip beside it.
pub fn panel_header(ui: &Ui, title: String) -> Div {
    div()
        .flex()
        .items_center()
        .flex_none()
        .h_9()
        .px_3()
        .border_b_1()
        .border_color(ui.pal.border)
        .bg(ui.pal.panel)
        .text_xs()
        .font_family(ui.mono.clone())
        .child(title)
}

pub fn nav(ui: &Ui, vm: &AppVm) -> Div {
    let pal = &ui.pal;
    let title = "Navigate";

    div()
        .flex()
        .flex_col()
        .size_full()
        .border_r_1()
        .border_color(pal.border)
        .bg(pal.panel)
        .child(panel_header(ui, title.to_string()))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .p_2()
                .children(vm.nav.iter().map(|s| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .when(!s.title.is_empty(), |d| {
                            d.child(
                                div()
                                    .px_2()
                                    .pb_1()
                                    .text_xs()
                                    .text_color(pal.faint)
                                    .child(s.title.clone()),
                            )
                        })
                        .children(s.items.iter().map(|i| {
                            div()
                                .id(SharedString::from(format!("nav-{}", i.label)))
                                .flex()
                                .items_center()
                                .justify_between()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .text_sm()
                                .cursor_pointer()
                                .bg(if i.selected {
                                    pal.accent.opacity(0.16)
                                } else {
                                    gpui_kit::transparent_black()
                                })
                                .text_color(if i.selected { pal.fg } else { pal.muted })
                                .hover(|s| s.bg(pal.hover))
                                .on_click(ui.on_click(Intent::Go(i.route.clone())))
                                .child(i.label.clone())
                                .when(i.count > 0, |d| {
                                    d.child(
                                        div()
                                            .text_xs()
                                            .text_color(pal.faint)
                                            .child(i.count.to_string()),
                                    )
                                })
                        }))
                })),
        )
}

/* ------------------------------ tab strip ------------------------------ */

pub fn tabstrip(ui: &Ui, vm: &AppVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .items_end()
        .flex_none()
        .h_9()
        .border_b_1()
        .border_color(pal.border)
        .bg(pal.panel)
        .children(vm.tabs.iter().map(|t| {
            let go = Intent::Go(t.route.clone());
            let pin = t.key.clone().map(Intent::PinTab);
            let handler_ui = ui.clone();
            div()
                .id(SharedString::from(format!("tab-{}", t.label)))
                .flex()
                .items_center()
                .gap_2()
                .h_full()
                .px_3()
                .text_sm()
                .cursor_pointer()
                .border_r_1()
                .border_color(pal.border)
                .bg(if t.selected {
                    pal.bg
                } else {
                    gpui_kit::transparent_black()
                })
                .text_color(if t.selected { pal.fg } else { pal.muted })
                .when(!t.pinned, |d| d.italic())
                .hover(|s| s.bg(pal.hover))
                .on_click(move |ev, _, cx| {
                    handler_ui.send(go.clone(), cx);
                    if ev.click_count() >= 2
                        && let Some(p) = pin.clone()
                    {
                        handler_ui.send(p, cx);
                    }
                })
                .when(t.active, |d| {
                    d.child(div().text_color(pal.accent).text_xs().child("▶"))
                })
                .when(!t.active && t.bad, |d| d.child(dot(pal.bad)))
                .when(!t.active && !t.bad && t.unseen, |d| {
                    d.child(dot(pal.accent))
                })
                .child(
                    div()
                        .font_family(ui.mono.clone())
                        .text_xs()
                        .child(t.label.clone()),
                )
                .when(!t.title.is_empty(), |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(pal.faint)
                            .child(short(&t.title, 18)),
                    )
                })
                .when_some(t.key.clone(), |d, key| {
                    d.child(
                        div()
                            .id(SharedString::from(format!("close-{key}")))
                            .px_1()
                            .rounded_sm()
                            .text_color(pal.faint)
                            .hover(|s| s.bg(pal.border).text_color(pal.fg))
                            // The tab under it selects on click; closing must not also select.
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(ui.on_click(Intent::CloseTab(key)))
                            .child("×"),
                    )
                })
        }))
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

/* ------------------------------ right panel ------------------------------ */

/// One row of the right panel, by kind.
fn right_row(ui: &Ui, r: &RightRow) -> AnyElement {
    let pal = &ui.pal;
    match r {
        RightRow::Kv(k) => kv(pal, k).into_any_element(),
        RightRow::Text(t) => div().text_sm().child(t.clone()).into_any_element(),
        RightRow::Muted(t) => faint(pal, t.clone()).into_any_element(),
        RightRow::Tags(tags) => pills(pal, tags).into_any_element(),
        // An item reads key, then title, then detail: three steps down in brightness.
        RightRow::Item {
            head,
            key,
            title,
            sub,
        } => div()
            .flex()
            .flex_col()
            .gap_0p5()
            .when(!head.is_empty() || key.is_some(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .children(head.iter().map(|b| pill(pal, b)))
                        .when_some(key.clone(), |d, k| {
                            d.child(key_link(
                                ui,
                                &k,
                                Intent::go_ticket(k.clone(), TicketTab::Overview),
                            ))
                        }),
                )
            })
            .child(div().text_sm().child(title.clone()))
            .when_some(sub.clone(), |d, s| d.child(faint(pal, s)))
            .into_any_element(),
        // One line, clickable as a whole: a mark for what is urgent, the key, then as much of the title as fits.
        RightRow::Ticket { key, title, mark } => {
            let full: SharedString = title.clone().into();
            div()
                .id(hash_id("right-ticket", key))
                .flex()
                .items_center()
                .gap_2()
                .h(px(28.0))
                .px_2()
                .mx_neg_2()
                .rounded_md()
                .cursor_pointer()
                .hover(|d| d.bg(pal.hover))
                .on_click(ui.on_click(Intent::go_ticket(key.clone(), TicketTab::Overview)))
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(full.clone()).build(window, cx)
                })
                .child(
                    div()
                        .flex_none()
                        .w(px(14.0))
                        .children(mark.as_ref().map(|m| {
                            Icon::new(IconName::TriangleAlert)
                                .small()
                                .text_color(pal.tone(m.tone))
                        })),
                )
                .child(key_text(ui, key))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(pal.muted)
                        .child(title.clone()),
                )
                .into_any_element()
        }
        RightRow::Activity { at, text, failed } => div()
            .flex()
            .gap_2()
            .text_xs()
            .child(
                div()
                    .flex_none()
                    .font_family(ui.mono.clone())
                    .text_color(pal.faint)
                    .child(at.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(pal.muted)
                    .child(text.clone()),
            )
            .when(*failed, |d| {
                d.child(pill(pal, &Badge::new("failed", Tone::Bad)))
            })
            .into_any_element(),
        RightRow::Report { ok, source, text } => div()
            .flex()
            .gap_2()
            .text_xs()
            .child(dot(if *ok { pal.ok } else { pal.bad }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(pal.muted)
                    .child(format!("{source}: {text}")),
            )
            .into_any_element(),
        RightRow::Button(a) => div().flex().child(button(ui, a)).into_any_element(),
    }
}

/// The right dock: a header strip, then sections grouped by hairlines rather than boxes.
pub fn right_panel(ui: &Ui, title: &str, sections: &[RightSection]) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .size_full()
        .border_l_1()
        .border_color(pal.border)
        .bg(pal.panel)
        .child(panel_header(ui, title.to_string()))
        .child(
            div()
                .id("right-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .px_3()
                .pb_3()
                .overflow_y_scroll()
                .children(sections.iter().enumerate().map(|(n, s)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .py_3()
                        .when(n > 0, |d| {
                            d.border_t_1().border_color(pal.border.opacity(0.6))
                        })
                        .child(div().text_xs().text_color(pal.muted).child(s.title.clone()))
                        .children(s.rows.iter().map(|r| right_row(ui, r)))
                })),
        )
}

/* ------------------------------ status bar ------------------------------ */

pub fn status_bar(ui: &Ui, s: &StatusVm) -> Div {
    let pal = &ui.pal;
    let health = |name: &str, ok: bool| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(dot(if ok { pal.ok } else { pal.warn }))
            .child(name.to_string())
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .h_7()
        .flex_none()
        .px_3()
        .border_t_1()
        .border_color(pal.border)
        .bg(pal.panel)
        .text_xs()
        .text_color(pal.muted)
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .when_some(s.active.clone(), |d, a| {
                    d.child(
                        div()
                            .id("status-active")
                            .flex()
                            .gap_1()
                            .cursor_pointer()
                            .text_color(pal.accent)
                            .on_click(ui.on_click(Intent::go_ticket(&a.key, TicketTab::Test)))
                            .child(format!("▶ {}", a.key))
                            .child(div().font_family(ui.mono.clone()).child(a.elapsed.clone())),
                    )
                    .child(
                        div()
                            .id("status-park")
                            .cursor_pointer()
                            .hover(|s| s.text_color(pal.fg))
                            .on_click(ui.on_click(Intent::Do(Command::Park(a.key.clone()))))
                            .child("Park…"),
                    )
                })
                .when_some(s.overlay.clone(), |d, o| {
                    d.child(pill(pal, &Badge::new(format!("overlay: {o}"), Tone::Warn)))
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .children(s.chips.iter().map(|c| pill(pal, c)))
                .child(health("Jira", s.jira_ready))
                .child(health("GitHub", s.gh_ready))
                .child(
                    div()
                        .id("status-sync")
                        .cursor_pointer()
                        .text_color(pal.tone(s.sync_tone))
                        .on_click(ui.on_click(Intent::Do(Command::Sync)))
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(s.sync_text.clone())
                        // The icon turns while a sync runs, and is the plain refresh mark otherwise.
                        .child(if s.sync_running {
                            gpui_kit::component::spinner::Spinner::new()
                                .xsmall()
                                .color(pal.tone(s.sync_tone))
                                .into_any_element()
                        } else {
                            div().child("↻").into_any_element()
                        }),
                ),
        )
}

/* ------------------------------ overlays ------------------------------ */

/// A dimmed backdrop that swallows clicks and does nothing with them.
fn scrim_inert(ui: &Ui) -> Stateful<Div> {
    div()
        .id("scrim-inert")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(ui.pal.bg.opacity(0.7))
}

fn scrim(ui: &Ui, intent: Intent, dim: bool) -> Stateful<Div> {
    div()
        .id("scrim")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .bg(if dim {
            ui.pal.bg.opacity(0.7)
        } else {
            gpui_kit::transparent_black()
        })
        .on_click(ui.on_click(intent))
}

pub fn attention_panel(ui: &Ui, a: &AttentionVm) -> Div {
    let pal = &ui.pal;
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(scrim(ui, Intent::ToggleAttention, false))
        .child(
            div()
                .absolute()
                .top(px(40.0))
                .right(px(12.0))
                .w(px(460.0))
                .max_h(px(560.0))
                .occlude()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(pal.border)
                .bg(pal.bg)
                .shadow_lg()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().child("Needs attention"))
                        .child(button_ghost(
                            ui,
                            &Btn::new("All next actions", Intent::Go(Route::Next)),
                        )),
                )
                .child(if a.items.is_empty() {
                    muted(pal, "Nothing needs you right now.").into_any_element()
                } else {
                    div()
                        .id("attn-list")
                        .flex()
                        .flex_col()
                        .gap_2()
                        .overflow_y_scroll()
                        .children(a.items.iter().map(|s| suggestion_row(ui, s)))
                        .into_any_element()
                }),
        )
}

pub fn simulate_panel(ui: &Ui, groups: &[SimGroup]) -> Div {
    let pal = &ui.pal;
    div()
        .absolute()
        .top(px(44.0))
        .left(px(112.0))
        .w(px(280.0))
        .occlude()
        .flex()
        .flex_col()
        .gap_3()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(pal.border)
        .bg(pal.bg)
        .shadow_lg()
        .child(div().child("Simulate the outside world"))
        .children(groups.iter().map(|g| {
            section(pal, g.title.clone()).child(
                div().flex().flex_col().gap_1().children(
                    g.buttons
                        .iter()
                        .map(|b| div().child(check_or_button(ui, b))),
                ),
            )
        }))
}

fn check_or_button(ui: &Ui, b: &SimButton) -> AnyElement {
    chip(ui, b.label.clone(), b.on, b.intent.clone()).into_any_element()
}

pub fn toasts(ui: &Ui, toasts: &[Toast]) -> Div {
    let pal = &ui.pal;
    div()
        .absolute()
        .bottom(px(40.0))
        .right(px(16.0))
        .flex()
        .flex_col()
        .gap_2()
        .children(toasts.iter().map(|t| {
            let c = match t.kind {
                ToastKind::Info => pal.accent,
                ToastKind::Ok => pal.ok,
                ToastKind::Warn => pal.warn,
                ToastKind::Bad => pal.bad,
            };
            div()
                .flex()
                .items_center()
                .gap_3()
                .max_w(px(420.0))
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(c.opacity(0.6))
                .bg(pal.bg)
                .shadow_lg()
                .text_sm()
                .child(dot(c))
                .child(div().flex_1().child(t.text.clone()))
                .when_some(t.undo.clone(), |d, u| {
                    d.child(button_ghost(ui, &Btn::new("Undo", u)))
                })
        }))
}

/* ------------------------------ sheets ------------------------------ */

fn sheet_frame(ui: &Ui, width: f32, title: Div, body: Div, footer: Div) -> Div {
    sheet_frame_with(ui, width, title, body, footer, Some(Intent::CancelSheet))
}

/// A modal frame. `dismiss` is what a click outside it does; `None` makes the modal impossible to leave by clicking
/// away (a sequence that is running).
fn sheet_frame_with(
    ui: &Ui,
    width: f32,
    title: Div,
    body: Div,
    footer: Div,
    dismiss: Option<Intent>,
) -> Div {
    let pal = &ui.pal;
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(match dismiss {
            Some(intent) => scrim(ui, intent, true).into_any_element(),
            None => scrim_inert(ui).into_any_element(),
        })
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_start()
                .justify_center()
                .pt(px(72.0))
                .child(
                    div()
                        .occlude()
                        .w(px(width))
                        .max_h(px(620.0))
                        .flex()
                        .flex_col()
                        .rounded_xl()
                        .border_1()
                        .border_color(pal.border)
                        .bg(pal.bg)
                        .shadow_lg()
                        .child(
                            div()
                                .px_4()
                                .py_3()
                                .border_b_1()
                                .border_color(pal.border)
                                .child(title),
                        )
                        .child(
                            div()
                                .id("sheet-body")
                                .flex()
                                .flex_col()
                                .gap_3()
                                .p_4()
                                .overflow_y_scroll()
                                .child(body),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_end()
                                .gap_2()
                                .px_4()
                                .py_3()
                                .border_t_1()
                                .border_color(pal.border)
                                .child(footer),
                        ),
                ),
        )
}

fn risk_banner(ui: &Ui, p: &Preview) -> Div {
    let pal = &ui.pal;
    let (tone, label) = match p.risk {
        Risk::High => (Tone::Bad, "⛔ HIGH RISK"),
        Risk::Medium => (Tone::Warn, "MEDIUM RISK"),
        Risk::Low => (Tone::Neutral, "LOW RISK"),
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(pill(pal, &Badge::new(label, tone)))
                .child(
                    div()
                        .when(p.risk == Risk::High, |d| d.font_weight(FontWeight::BOLD))
                        .child(p.title.clone()),
                ),
        )
        .when_some(p.ticket.clone(), |d, k| d.child(key_text(ui, &k)))
}

fn confirm_sheet(ui: &Ui, cx: &App, inputs: &Inputs, preview: &Preview, can_confirm: bool) -> Div {
    let pal = &ui.pal;
    let cancel = button_ghost(ui, &Btn::new("Cancel", Intent::CancelSheet));
    if let Some(why) = &preview.blocked {
        return sheet_frame(
            ui,
            560.0,
            risk_banner(ui, preview),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(warn_box(
                    pal,
                    Tone::Bad,
                    [div().child(why.clone()).into_any_element()],
                ))
                .child(muted(
                    pal,
                    "Nothing was sent. Fix the sign-in and try again.",
                )),
            div().child(cancel),
        );
    }
    let body = div()
        .flex()
        .flex_col()
        .gap_3()
        .child(div().text_sm().child(preview.summary.clone()))
        .when(!preview.payload.is_empty(), |d| {
            d.child(section(pal, "This will run, exactly").child(code_block(
                pal,
                cx,
                preview.payload.join("\n"),
            )))
        })
        .when(!preview.facts.is_empty(), |d| {
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(preview.facts.iter().map(|f| {
                        div()
                            .font_family(ui.mono.clone())
                            .text_xs()
                            .text_color(pal.muted)
                            .child(f.clone())
                    })),
            )
        })
        .when_some(preview.type_key.clone(), |d, k| {
            d.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().child(format!("Type {k} to confirm:")))
                    .child(inputs.line(&Field::TypedKey)),
            )
        });
    sheet_frame(
        ui,
        640.0,
        risk_banner(ui, preview),
        body,
        div().flex().gap_2().child(cancel).child(
            button(
                ui,
                &Btn::new(preview.confirm_label.clone(), Intent::ConfirmSheet).primary(),
            )
            .disabled(!can_confirm),
        ),
    )
}

pub fn sheet(ui: &Ui, cx: &App, inputs: &Inputs, sheet: &SheetVm) -> Div {
    let pal = &ui.pal;
    match sheet {
        SheetVm::Confirm { preview, can_confirm, .. } => confirm_sheet(ui, cx, inputs, preview, *can_confirm),
        SheetVm::Baseline(b) => sheet_frame(
            ui,
            520.0,
            div().child(format!("{} is a hotfix", b.key)),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(muted(pal, "Untouched repos should run on:"))
                .children(b.choices.iter().map(|(choice, label)| {
                    div().flex().child(button(
                        ui,
                        &Btn::new(label.clone(), Intent::PickBaseline(*choice)).primary_if(*choice == Baseline::Develop),
                    ))
                }))
                .child(faint(pal, "Applies to this activation only.")),
            div().child(button_ghost(ui, &Btn::new("Cancel", Intent::CancelSheet))),
        ),
        SheetVm::Report(r) => sheet_frame(
            ui,
            560.0,
            div().child(r.title.clone()),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .children(r.rows.iter().map(|row| {
                    div()
                        .flex()
                        .gap_3()
                        .text_sm()
                        .child(div().text_color(if row.ok { pal.ok } else { pal.bad }).child(if row.ok { "✓" } else { "✕" }))
                        .child(div().w(px(100.0)).flex_none().font_family(ui.mono.clone()).child(row.repo.to_string()))
                        .child(div().flex_1().min_w_0().child(row.detail.clone()))
                }))
                .children(r.notes.iter().map(|n| faint(pal, n.clone()))),
            div().flex().child(button(ui, &Btn::new("Done", Intent::CancelSheet).primary())),
        ),
        SheetVm::Compose { title, placeholder, confirm_label, text, .. } => sheet_frame(
            ui,
            560.0,
            div().child(title.clone()),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(faint(pal, placeholder.clone()))
                .child(inputs.area(&Field::Compose)),
            div()
                .flex()
                .gap_2()
                .child(button_ghost(ui, &Btn::new("Cancel", Intent::CancelSheet)))
                .child(
                    button(ui, &Btn::new(confirm_label.clone(), Intent::Submit(Field::Compose)).primary())
                        .disabled(text.trim().is_empty()),
                ),
        ),
        SheetVm::Palette { items, .. } => div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(scrim(ui, Intent::ClosePalette, true))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .justify_center()
                    .items_start()
                    .pt(px(96.0))
                    .child(
                        div()
                            .occlude()
                            .w(px(560.0))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .p_2()
                            .rounded_xl()
                            .border_1()
                            .border_color(pal.border)
                            .bg(pal.bg)
                            .shadow_lg()
                            .child(inputs.line(&Field::Palette))
                            .children(items.iter().map(|i| {
                                div()
                                    .id(SharedString::from(format!("pal-{}", i.label)))
                                    .flex()
                                    .justify_between()
                                    .px_3()
                                    .py_1p5()
                                    .rounded_md()
                                    .text_sm()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(pal.hover))
                                    .on_click(ui.on_click(i.intent.clone()))
                                    .child(i.label.clone())
                                    .child(faint(pal, i.hint.clone()))
                            })),
                    ),
            ),
        SheetVm::ClaimBlocked { key, block, then } => {
            let park = Btn::new(
                format!("Park {} and continue", block.blocker),
                Intent::Do(Command::ParkAndContinue {
                    from: block.blocker.clone(),
                    key: key.clone(),
                    then: *then,
                }),
            )
            .primary();
            sheet_frame(
                ui,
                520.0,
                div().child("One ticket at a time"),
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().child(format!(
                        "{} {} is {}. Finish it, or park it to take {key} now.",
                        block.blocker, block.title, block.what
                    )))
                    .child(faint(
                        pal,
                        if block.active {
                            "Parking restores your repos and reverts the overlay. Its notes and checklist stay."
                        } else {
                            "Parking keeps its notes and checklist. You can pick it up again later."
                        },
                    )),
                div()
                    .flex()
                    .gap_2()
                    .child(button_ghost(ui, &Btn::new("Cancel", Intent::CancelSheet)))
                    .child(button(ui, &park)),
            )
        }
        SheetVm::Busy { busy, retry } => sheet_frame(
            ui,
            520.0,
            div().child(format!("{} is {}", busy.repo, if busy.stale { "locked" } else { "busy" })),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().child(busy.message.clone()))
                .child(faint(
                    pal,
                    if busy.stale {
                        "A crashed git or editor leaves this behind. The app never removes it without asking."
                    } else {
                        "One thing at a time per repo, so the working tree, stash and branches cannot be changed from two places."
                    },
                )),
            div()
                .flex()
                .gap_2()
                .child(button_ghost(ui, &Btn::new("Close", Intent::CancelSheet)))
                .child(if busy.stale {
                    button(ui, &Btn::new("Remove lock…", Intent::Do(Command::BreakLock(busy.repo.clone()))).primary())
                } else {
                    button(ui, &Btn::new("Retry", Intent::Do(retry.clone())).primary())
                }),
        ),
        SheetVm::Diagnose(text) => sheet_frame(
            ui,
            680.0,
            div().child("Diagnose providers (fake output)"),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(warn_box(
                    pal,
                    Tone::Warn,
                    [div()
                        .child("Real output can contain ticket titles and names. Read it before sharing.")
                        .into_any_element()],
                ))
                .child(code_block(pal, cx, text.clone())),
            div().flex().child(button(ui, &Btn::new("Close", Intent::CancelSheet).primary())),
        ),
        SheetVm::AllWorkspaces(a) => sheet_frame(
            ui,
            520.0,
            div().child("All workspaces"),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(inputs.line(&Field::WorkspaceSearch))
                .child(
                    div()
                        .id("all-workspaces")
                        .flex()
                        .flex_col()
                        .max_h(px(360.0))
                        .overflow_y_scroll()
                        .children(a.items.iter().map(|w| all_workspaces_row(ui, w)))
                        .when(a.items.is_empty(), |d| {
                            d.child(div().py_3().child(muted(
                                pal,
                                if a.total == 0 {
                                    "No workspaces yet.".to_string()
                                } else {
                                    format!("Nothing matches \u{201c}{}\u{201d}.", a.query.trim())
                                },
                            )))
                        }),
                ),
            div().child(button_ghost(ui, &Btn::new("Close", Intent::CancelSheet))),
        ),
        SheetVm::CloseTab(key) => sheet_frame(
            ui,
            460.0,
            div().child(format!("Close {key}?")),
            div().text_sm().child("You have an unsent comment on this ticket. Closing the tab discards it."),
            div()
                .flex()
                .gap_2()
                .child(button_ghost(ui, &Btn::new("Keep open", Intent::CancelSheet)))
                .child(button(ui, &Btn::new("Discard and close", Intent::ForceCloseTab(key.clone())))),
        ),
    }
}
