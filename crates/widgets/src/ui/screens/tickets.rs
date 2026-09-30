//! Ticket lists: one table for every group. A filter strip and a column header that stay put, a uniform list of
//! fixed-height rows under them.
//!
//! Only exceptions get colour or a badge: priority shows when it is High or Highest, the local state only on the
//! "All tickets" list (inside a group it is implied), Jira status is a dot and a word, flags are pills.

use std::rc::Rc;

use gpui_kit::component::{Icon, IconName, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::ctx::{Inputs, Ui};
use crate::ui::filter_menu::{FilterGroup, FilterRow, filter_menu};
use crate::ui::widgets::*;
use crate::vm::*;

const ROW_H: f32 = 52.0;
const KEY_W: f32 = 90.0;
const URGENT_W: f32 = 24.0;
const REPOS_W: f32 = 140.0;
const JIRA_W: f32 = 120.0;
const LOCAL_W: f32 = 84.0;
const FLAGS_W: f32 = 170.0;

fn cell(w: f32) -> Div {
    div().flex_none().w(px(w)).px_2()
}

/// A column header; the sortable ones are buttons, the active one shows its direction.
fn head_cell(
    ui: &Ui,
    vm: &TicketListVm,
    label: &'static str,
    col: Option<TicketSort>,
) -> Stateful<Div> {
    let pal = &ui.pal;
    let active = col.and_then(|c| vm.sort.filter(|(s, _)| *s == c));
    let mut d = div()
        .id(SharedString::from(format!("th-{label}")))
        .flex()
        .items_center()
        .gap_1()
        .text_xs()
        .text_color(if active.is_some() { pal.fg } else { pal.faint })
        .child(label.to_uppercase());
    if let Some((_, asc)) = active {
        d = d.child(
            Icon::new(if asc {
                IconName::ArrowUp
            } else {
                IconName::ArrowDown
            })
            .xsmall(),
        );
    }
    if let Some(c) = col {
        d = d
            .cursor_pointer()
            .on_click(ui.on_click(Intent::SortTickets(c)));
    }
    d
}

fn header(ui: &Ui, vm: &TicketListVm, show_local: bool) -> Div {
    let pal = &ui.pal;
    div()
        .flex()
        .flex_none()
        .items_center()
        .px_4()
        .py_1()
        .border_b_1()
        .border_color(pal.border.opacity(0.6))
        .child(cell(KEY_W).child(head_cell(ui, vm, "Key", Some(TicketSort::Key))))
        .child(cell(URGENT_W))
        .child(div().flex_1().min_w_0().px_2().child(head_cell(
            ui,
            vm,
            "Title",
            Some(TicketSort::Title),
        )))
        .child(cell(REPOS_W).child(head_cell(ui, vm, "Repos", None)))
        .child(cell(JIRA_W).child(head_cell(ui, vm, "Jira", Some(TicketSort::Jira))))
        .when(show_local, |d| {
            d.child(cell(LOCAL_W).child(head_cell(ui, vm, "Local", None)))
        })
        .child(cell(FLAGS_W).child(head_cell(ui, vm, "Flags", None)))
}

/// One ticket row.
pub fn ticket_row(ui: &Ui, r: &TicketRowVm, show_local: bool) -> Stateful<Div> {
    let pal = &ui.pal;
    let intent = Intent::go_ticket(r.key.clone(), TicketTab::Overview);
    let tone = pal.tone(r.jira.tone);
    div()
        .id(SharedString::from(format!("row-{}", r.key)))
        .flex()
        .items_center()
        .w_full()
        .h(px(ROW_H))
        .px_4()
        .border_b_1()
        .border_color(pal.border.opacity(0.5))
        .cursor_pointer()
        .hover(|s| s.bg(pal.hover))
        .on_click(ui.on_click(intent))
        .child(cell(KEY_W).child(key_text(ui, &r.key)))
        .child(cell(URGENT_W).when(r.urgent, |d| {
            d.child(
                Icon::new(IconName::TriangleAlert)
                    .small()
                    .text_color(pal.tone(r.priority.tone)),
            )
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .truncate()
                                .when(r.flags.iter().any(|b| b.tone == Tone::Bad), |d| {
                                    d.font_weight(FontWeight::BOLD)
                                })
                                .child(r.title.clone()),
                        )
                        .when(r.hotfix, |d| d.child(hotfix_pill(pal))),
                )
                .child(div().truncate().child(faint(pal, r.sub.clone()))),
        )
        .child(
            cell(REPOS_W)
                .truncate()
                .text_xs()
                .font_family(ui.mono.clone())
                .text_color(pal.muted)
                .child(
                    r.repos
                        .iter()
                        .map(|x| x.to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
        )
        .child(
            cell(JIRA_W).child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(pal.muted)
                    .child(dot(if r.jira.tone == Tone::Neutral {
                        pal.faint
                    } else {
                        tone
                    }))
                    .child(r.jira.text.clone()),
            ),
        )
        .when(show_local, |d| {
            d.child(
                cell(LOCAL_W)
                    .text_xs()
                    .text_color(pal.muted)
                    .child(match &r.local {
                        Some(b) => b.text.clone(),
                        None => "unclaimed".to_string(),
                    }),
            )
        })
        .child(
            cell(FLAGS_W)
                .flex()
                .gap_1()
                .overflow_hidden()
                .children(r.flags.iter().map(|b| pill(pal, b))),
        )
}

/// What the list shows, flattened so every item has the same height.
enum Item {
    Heading(String),
    Ticket(TicketRowVm),
}

fn filter_groups(vm: &TicketListVm) -> Vec<FilterGroup> {
    let on = |f: &TicketFilter| vm.filters.is_on(f);
    let mut groups = Vec::new();
    if !vm.options.repos.is_empty() {
        groups.push(FilterGroup {
            title: "Repos",
            rows: vm
                .options
                .repos
                .iter()
                .map(|r| {
                    let f = TicketFilter::Repo(r.clone());
                    FilterRow::new(r.to_string(), on(&f), Intent::ToggleTicketFilter(f))
                })
                .collect(),
        });
    }
    if vm.options.jira.len() > 1 || !vm.filters.jira.is_empty() {
        groups.push(FilterGroup {
            title: "Jira status",
            rows: vm
                .options
                .jira
                .iter()
                .map(|j| {
                    let f = TicketFilter::Jira(j.clone());
                    FilterRow::new(j.clone(), on(&f), Intent::ToggleTicketFilter(f))
                })
                .collect(),
        });
    }
    if vm.options.priority.len() > 1 || !vm.filters.priority.is_empty() {
        groups.push(FilterGroup {
            title: "Priority",
            rows: vm
                .options
                .priority
                .iter()
                .map(|p| {
                    let f = TicketFilter::Priority(p.clone());
                    FilterRow::new(p.clone(), on(&f), Intent::ToggleTicketFilter(f))
                })
                .collect(),
        });
    }
    groups.push(FilterGroup {
        title: "Flags",
        rows: vec![
            FilterRow::new(
                "Hotfix",
                vm.filters.hotfix,
                Intent::ToggleTicketFilter(TicketFilter::Hotfix),
            ),
            FilterRow::new(
                "New comments",
                vm.filters.new_comments,
                Intent::ToggleTicketFilter(TicketFilter::NewComments),
            ),
        ],
    });
    groups
}

pub fn tickets(ui: &Ui, inputs: &Inputs, vm: &TicketListVm) -> AnyElement {
    let pal = &ui.pal;
    let show_local = vm.group == Group::All;
    let items: Vec<Item> = vm
        .sections
        .iter()
        .flat_map(|s| {
            s.heading
                .iter()
                .map(|h| Item::Heading(h.clone()))
                .chain(s.rows.iter().cloned().map(Item::Ticket))
                .collect::<Vec<_>>()
        })
        .collect();
    let strip = div()
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
                .child(inputs.search(&Field::TicketFilter)),
        )
        .child(filter_menu(
            ui,
            "ticket-filters",
            filter_groups(vm),
            Some(Intent::ClearTicketFilters),
        ));
    let body = if items.is_empty() {
        let (text, link) = if vm.total > 0 {
            ("No ticket here matches.".to_string(), None)
        } else {
            (
                format!("Nothing in {}.", vm.group.label().to_lowercase()),
                vm.elsewhere.map(|(g, n)| {
                    Btn::new(
                        format!("{n} in {}", g.label().to_lowercase()),
                        Intent::go_tickets(g),
                    )
                }),
            )
        };
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(empty_state(pal, text))
            .when_some(link, |d, a| d.child(button_ghost(ui, &a)))
            .into_any_element()
    } else {
        let items = Rc::new(items);
        let list_ui = ui.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(header(ui, vm, show_local))
            .child(
                uniform_list("ticket-list", items.len(), move |range, _, _| {
                    range
                        .map(|i| match &items[i] {
                            Item::Heading(h) => div()
                                .flex()
                                .items_end()
                                .h(px(ROW_H))
                                .px_6()
                                .pb_1()
                                .child(h.clone())
                                .into_any_element(),
                            Item::Ticket(r) => {
                                ticket_row(&list_ui, r, show_local).into_any_element()
                            }
                        })
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
        .child(strip)
        .child(body)
        .into_any_element()
}
