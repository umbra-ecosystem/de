//! The window chrome: toolbar, navigation, tab strip, right panel, status bar, and the overlays above everything.

use gpui_kit::component::badge::Badge as CountBadge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, IconName, Selectable, Sizable};
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
        .label("Simulate")
        .selected(simulate_on)
        .on_click(ui.on_click(Intent::ToggleSimulate));

    div()
        .flex()
        .items_center()
        .gap_2()
        .w_full()
        .pr_3()
        .child(div().child("de"))
        .child(div().flex_1())
        .child(attention)
        .child(simulate)
        .child(panel)
}

/* ------------------------------ navigation ------------------------------ */

pub fn nav(ui: &Ui, vm: &AppVm) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_4()
        .size_full()
        .p_2()
        .border_r_1()
        .border_color(pal.border)
        .bg(pal.panel)
        .children(vm.nav.iter().map(|s| {
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_xs()
                        .text_color(pal.faint)
                        .child(s.title.to_uppercase()),
                )
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
        }))
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

pub fn right_panel(ui: &Ui, sections: &[RightSection]) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_col()
        .gap_4()
        .w_full()
        .p_3()
        .border_l_1()
        .border_color(pal.border)
        .bg(pal.panel)
        .children(sections.iter().map(|s| {
            section(pal, s.title.clone()).child(div().flex().flex_col().gap_2().children(
                s.rows.iter().map(|r| {
                    match r {
                        RightRow::Kv(k) => kv(pal, k).into_any_element(),
                        RightRow::Text(t) => div().text_sm().child(t.clone()).into_any_element(),
                        RightRow::Muted(t) => muted(pal, t.clone()).into_any_element(),
                        RightRow::Item {
                            head,
                            key,
                            title,
                            sub,
                        } => div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
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
                                            Intent::go_ticket(&k, TicketTab::Overview),
                                        ))
                                    }),
                            )
                            .child(div().text_sm().child(title.clone()))
                            .when_some(sub.clone(), |d, s| d.child(faint(pal, s)))
                            .into_any_element(),
                        RightRow::Activity { at, text, failed } => div()
                            .flex()
                            .gap_2()
                            .text_xs()
                            .child(
                                div()
                                    .font_family(ui.mono.clone())
                                    .text_color(pal.faint)
                                    .child(at.clone()),
                            )
                            .child(div().child(text.clone()))
                            .when(*failed, |d| {
                                d.child(pill(pal, &Badge::new("failed", Tone::Bad)))
                            })
                            .into_any_element(),
                        RightRow::Report { ok, source, text } => div()
                            .flex()
                            .gap_2()
                            .text_xs()
                            .child(dot(if *ok { pal.ok } else { pal.bad }))
                            .child(div().child(div().child(source.clone())).child(text.clone()))
                            .into_any_element(),
                        RightRow::Button(a) => div().child(button(ui, a)).into_any_element(),
                    }
                }),
            ))
        }))
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
                        .child(format!("{} ↻", s.sync_text)),
                ),
        )
}

/* ------------------------------ overlays ------------------------------ */

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
        .right(px(12.0))
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
    let pal = &ui.pal;
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(scrim(ui, Intent::CancelSheet, true))
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
                    div().child(button(
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
                        .child(div().child(row.detail.clone()))
                }))
                .children(r.notes.iter().map(|n| faint(pal, n.clone()))),
            div().child(button(ui, &Btn::new("Done", Intent::CancelSheet).primary())),
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
            div().child(button(ui, &Btn::new("Close", Intent::CancelSheet).primary())),
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
