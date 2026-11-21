//! A searchable dropdown in the select2 style (#92, #94): a button showing
//! the current choice; clicked, a popup with a search field on top, the
//! filtered list (↑ / ↓ move, Enter picks, Escape closes), an optional
//! trailing delete icon per row and footer items that stay visible while
//! filtering.

use eframe::egui::{self, Align2, Key, Modifiers, PopupCloseBehavior, RichText, Sense};
use telepad_app::{PickerDraft, matches_query};

use crate::icons::{self, label as il};
use crate::widgets::pal;

pub struct Item {
    pub icon: &'static str,
    pub label: String,
    /// The current choice: marked with a check.
    pub current: bool,
    /// Weak text after the label (e.g. "minimized").
    pub note: &'static str,
    /// Hover text of a trailing delete icon; `None`: no icon.
    pub delete: Option<String>,
}

pub struct Footer {
    pub icon: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    pub tip: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picked {
    Item(usize),
    Delete(usize),
    Footer(usize),
}

pub struct Spec<'a> {
    pub button: String,
    pub tip: &'a str,
    pub enabled: bool,
    pub placeholder: &'a str,
    pub empty: &'a str,
    pub width: f32,
}

/// Longer button texts end in "…" so the top bar keeps its place.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

pub fn picker(
    ui: &mut egui::Ui,
    spec: Spec<'_>,
    items: &[Item],
    footer: &[Footer],
    d: &mut PickerDraft,
) -> Option<Picked> {
    let resp = ui
        .add_enabled(
            spec.enabled,
            egui::Button::new(format!("{} {}", spec.button, icons::CARET_DOWN)),
        )
        .on_hover_text(spec.tip);
    let ctx = ui.ctx().clone();
    let popup_id = egui::Popup::default_response_id(&resp);
    let was_open = egui::Popup::is_id_open(&ctx, popup_id);
    let mut picked = None;
    let mut close = false;
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(spec.width)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .show(|ui| {
            ui.set_min_width(spec.width);
            // Opened: the current choice is highlighted, so Enter keeps it.
            if !was_open {
                d.query.clear();
                d.highlight = items.iter().position(|it| it.current).unwrap_or(0);
            }
            let shown: Vec<usize> = items
                .iter()
                .enumerate()
                .filter(|(_, it)| matches_query(&it.label, &d.query))
                .map(|(i, _)| i)
                .collect();
            // Keys first, so the search field does not take them.
            let (down, up, enter, esc) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            let last = shown.len().saturating_sub(1);
            if down {
                d.highlight = (d.highlight + 1).min(last);
            }
            if up {
                d.highlight = d.highlight.saturating_sub(1);
            }
            d.highlight = d.highlight.min(last);
            let search = ui.add(
                egui::TextEdit::singleline(&mut d.query)
                    .hint_text(format!("{} {}", icons::SEARCH, spec.placeholder))
                    .desired_width(f32::INFINITY),
            );
            if !was_open {
                search.request_focus();
            }
            if search.changed() {
                d.highlight = 0;
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt(popup_id.with("list"))
                .max_height(320.0)
                .show(ui, |ui| {
                    if shown.is_empty() {
                        ui.label(RichText::new(spec.empty).weak());
                    }
                    for (row, &i) in shown.iter().enumerate() {
                        if let Some(p) = item_row(ui, &items[i], i, row, d, down || up) {
                            picked = Some(p);
                        }
                    }
                });
            if !footer.is_empty() {
                ui.separator();
                for (i, f) in footer.iter().enumerate() {
                    if ui
                        .add_enabled(
                            f.enabled,
                            egui::Button::new(il(f.icon, f.label)).frame(false),
                        )
                        .on_hover_text(f.tip)
                        .clicked()
                    {
                        picked = Some(Picked::Footer(i));
                    }
                }
            }
            if enter && let Some(&i) = shown.get(d.highlight) {
                picked = Some(Picked::Item(i));
            }
            close = esc;
        });
    if picked.is_some() || close {
        egui::Popup::close_id(&ctx, popup_id);
    }
    picked
}

fn item_row(
    ui: &mut egui::Ui,
    it: &Item,
    i: usize,
    row: usize,
    d: &mut PickerDraft,
    keys_moved: bool,
) -> Option<Picked> {
    let h = ui.spacing().interact_size.y + 2.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), Sense::click());
    if resp.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
        d.highlight = row;
    }
    let hl = d.highlight == row;
    if hl && keys_moved {
        resp.scroll_to_me(None);
    }
    let v = ui.visuals();
    if hl {
        ui.painter()
            .rect_filled(rect, 3.0, v.selection.bg_fill.gamma_multiply(0.45));
    }
    let trailing = it.delete.is_some().then(|| {
        egui::Rect::from_min_size(egui::pos2(rect.right() - h, rect.top()), egui::vec2(h, h))
    });
    let text_rect = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(trailing.map_or(rect.right(), |t| t.left()), rect.bottom()),
    );
    let painter = ui.painter().with_clip_rect(text_rect);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let color = if it.current {
        v.strong_text_color()
    } else {
        v.text_color()
    };
    if it.current {
        painter.text(
            egui::pos2(rect.left() + 4.0, rect.center().y),
            Align2::LEFT_CENTER,
            icons::CHECK,
            font.clone(),
            color,
        );
    }
    let label = painter.text(
        egui::pos2(rect.left() + 22.0, rect.center().y),
        Align2::LEFT_CENTER,
        format!("{} {}", it.icon, it.label),
        font.clone(),
        color,
    );
    if !it.note.is_empty() {
        painter.text(
            egui::pos2(label.right() + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            it.note,
            egui::TextStyle::Small.resolve(ui.style()),
            v.weak_text_color(),
        );
    }
    if let (Some(t), Some(tip)) = (trailing, &it.delete) {
        let b = ui
            .put(
                t,
                egui::Button::new(RichText::new(icons::TRASH).color(pal().red)).frame(false),
            )
            .on_hover_text(tip);
        if b.clicked() {
            return Some(Picked::Delete(i));
        }
    }
    resp.clicked().then_some(Picked::Item(i))
}
