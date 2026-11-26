//! Editor window of a file window (#64), VS Code-like: tabs with an unsaved
//! dot, line numbers, current line, syntax colours, find / replace / go to
//! line, status bar (line:column, indentation, language, line endings,
//! encoding). One inner window of the workspace per host (#77); its tabs hold
//! files of this computer and of the host together.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, Key, KeyboardShortcut, Modifiers, RichText,
    text::{CCursor, CCursorRange, LayoutJob, TextFormat},
};
use telepad_app::editor::{Answer, Bar, EdDraft, EdMsg, EdTab, TabState, Unsaved};
use telepad_app::files::{FileSession, FmMsg, Side};
use telepad_app::workspace::{Win, WsMsg};
use telepad_app::{Drafts, Model, Msg};
use telepad_core::db::human_size;
use telepad_core::files::syntax::{self, Lang, Tok};
use telepad_core::files::text::{self, FindOpts};
use telepad_core::files::time_text;

use crate::icons::{self, label as il};
use crate::widgets::{Act, act_text, usage_color};

/// Larger texts are shown without colours (fast enough to type in).
const COLOUR_LIMIT: usize = 2 * 1024 * 1024;

fn send(out: &mut Vec<Msg>, id: u64, msg: EdMsg) {
    out.push(Msg::Files(FmMsg::Editor { id, msg }));
}

fn red() -> Color32 {
    usage_color(95.0)
}

fn place(s: &FileSession, side: Side) -> String {
    match side {
        Side::Remote if !s.host.is_local() => s.host.alias.clone(),
        _ => "this computer".into(),
    }
}

fn edit_id(win: u64, tab: u64) -> egui::Id {
    egui::Id::unique(("telepad-ed-text", win, tab))
}

/// Selection to apply to a tab's text next frame (char range, focus the text).
fn select_later(ctx: &egui::Context, win: u64, tab: u64, r: Range<usize>, focus: bool) {
    ctx.data_mut(|m| m.insert_temp(edit_id(win, tab).with("select"), (r, focus)));
}

/// The match find / replace is on. Kept here because the text field drops
/// its selection while the find field has the focus.
fn current(ctx: &egui::Context, win: u64, tab: u64) -> Option<Range<usize>> {
    ctx.data(|m| m.get_temp::<Range<usize>>(edit_id(win, tab).with("match")))
}

fn set_current(ctx: &egui::Context, win: u64, tab: u64, r: Option<Range<usize>>) {
    let k = edit_id(win, tab).with("match");
    ctx.data_mut(|m| match r {
        Some(r) => {
            m.insert_temp(k, r);
        }
        None => m.remove::<Range<usize>>(k),
    });
}

/// The current match, if it is still one of `list`.
fn current_in(
    ctx: &egui::Context,
    win: u64,
    tab: u64,
    list: &[Range<usize>],
) -> Option<Range<usize>> {
    current(ctx, win, tab).filter(|c| list.contains(c))
}

fn cursor_of(ctx: &egui::Context, win: u64, tab: u64) -> Option<Range<usize>> {
    let st = egui::TextEdit::load_state(ctx, edit_id(win, tab))?;
    let r = st.cursor.char_range()?;
    let (a, b) = (r.primary.index.0, r.secondary.index.0);
    Some(a.min(b)..a.max(b))
}

/// The question about unsaved files brings the editor window to the front
/// once (it may be minimized or behind others).
pub fn raise_on_question(ctx: &egui::Context, m: &Model, out: &mut Vec<Msg>) {
    for s in &m.files {
        let asked = egui::Id::unique(("telepad-ed-asked", s.id));
        if s.editor.ask.is_some() {
            if !ctx.data(|m| m.get_temp::<bool>(asked).unwrap_or(false)) {
                out.push(Msg::Ws(WsMsg::Open(Win::Editor(s.id))));
                ctx.data_mut(|m| m.insert_temp(asked, true));
            }
        } else {
            ctx.data_mut(|m| m.remove::<bool>(asked));
        }
    }
}

/// One host's editor: an inner window of the workspace. Its shortcuts work
/// only while it is the top window (`focused`).
pub fn editor_page(
    ui: &mut egui::Ui,
    m: &Model,
    d: &mut Drafts,
    id: u64,
    focused: bool,
    out: &mut Vec<Msg>,
) {
    let Some(s) = m.files.iter().find(|s| s.id == id) else {
        return;
    };
    let Some(dr) = d.files.get_mut(&s.id) else {
        return;
    };
    if s.editor.tabs.is_empty() {
        return;
    }
    window(ui, s, &mut dr.editor, focused, out);
    question(ui, s, &dr.editor, out);
}

fn window(ui: &mut egui::Ui, s: &FileSession, dr: &mut EdDraft, focused: bool, out: &mut Vec<Msg>) {
    let id = s.id;
    let e = &s.editor;
    let Some(t) = e.active.and_then(|a| e.tab(a)).or(e.tabs.first()) else {
        return;
    };
    let tid = t.id;
    let ready = t.state == TabState::Ready;
    // Shortcuts first: the text field would take Ctrl+H / Ctrl+W as edits.
    if focused && e.ask.is_none() {
        let sc = |m: Modifiers, k: Key| {
            ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
        };
        if sc(Modifiers::COMMAND, Key::S) {
            send(out, id, EdMsg::Save(tid));
        }
        if sc(Modifiers::COMMAND, Key::W) {
            send(out, id, EdMsg::Close(tid));
        }
        if ready {
            let find = sc(Modifiers::COMMAND, Key::F);
            let replace = sc(Modifiers::COMMAND, Key::H);
            let goto = sc(Modifiers::COMMAND, Key::G);
            if find || replace {
                dr.bar = if replace { Bar::Replace } else { Bar::Find };
                // A selection on one line becomes the search text.
                if let (Some(r), Some(txt)) = (cursor_of(ui.ctx(), id, tid), dr.texts.get(&tid))
                    && !r.is_empty()
                {
                    let (a, b) = (
                        text::char_to_byte(txt, r.start),
                        text::char_to_byte(txt, r.end),
                    );
                    if !txt[a..b].contains('\n') {
                        dr.find = txt[a..b].to_string();
                    }
                }
                focus_field(ui.ctx(), id, "find");
            }
            if goto {
                dr.bar = Bar::Goto;
                dr.goto.clear();
                focus_field(ui.ctx(), id, "goto");
            }
            if dr.bar != Bar::None && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            {
                dr.bar = Bar::None;
                // Back in the text with the current match selected.
                match current(ui.ctx(), id, tid) {
                    Some(r) => select_later(ui.ctx(), id, tid, r, true),
                    None => ui.memory_mut(|m| m.request_focus(edit_id(id, tid))),
                }
                set_current(ui.ctx(), id, tid, None);
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F3)) {
                step(ui.ctx(), id, tid, dr, true);
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F3)) {
                step(ui.ctx(), id, tid, dr, false);
            }
        }
    }
    // Diff of the file window (#131): both files side by side.
    if let Some((a, b)) = e.compare
        && (tid == a || tid == b)
        && let (Some(ta), Some(tb)) = (e.tab(a), e.tab(b))
    {
        egui::Panel::top(("ed-tabs", id)).show(ui, |ui| {
            egui::ScrollArea::horizontal()
                .id_salt(("ed-tabs-scroll", id))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for x in &e.tabs {
                            tab_button(ui, s, x, x.id == a || x.id == b, dr, out);
                        }
                    });
                });
        });
        compare_view(ui, s, ta, tb, dr, out);
        return;
    }
    egui::Panel::top(("ed-tabs", id)).show(ui, |ui| {
        egui::ScrollArea::horizontal()
            .id_salt(("ed-tabs-scroll", id))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for x in &e.tabs {
                        tab_button(ui, s, x, Some(x.id) == Some(tid), dr, out);
                    }
                });
            });
    });
    egui::Panel::top(("ed-head", id)).show(ui, |ui| {
        // Editing who may log in (#85): the lockout risk is said up front.
        if t.side == Side::Remote && t.path.ends_with("/.ssh/authorized_keys") {
            ui.colored_label(
                usage_color(80.0),
                format!(
                    "{} authorized_keys: removing or breaking the key Telepad logs in with locks                      Telepad (and you) out of this host. Saved only when you click Save.",
                    icons::WARNING
                ),
            );
        }
        ui.horizontal(|ui| {
            let icon = match t.side {
                Side::Local => icons::DESKTOP,
                Side::Remote => icons::DISKS,
            };
            let full = format!("{icon} {}: {}", place(s, t.side), t.path);
            let w = (ui.available_width() - 160.0).max(80.0);
            ui.allocate_ui_with_layout(
                egui::vec2(w, ui.spacing().interact_size.y),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(egui::Label::new(RichText::new(&full).monospace().weak()).truncate())
                        .on_hover_text(&t.path);
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dirty = dr.dirty(t);
                if ui
                    .add_enabled(ready, egui::Button::new(il(icons::SEARCH, "Find")).small())
                    .on_hover_text("Find (Ctrl+F), replace (Ctrl+H), go to line (Ctrl+G)")
                    .clicked()
                {
                    dr.bar = Bar::Find;
                    focus_field(ui.ctx(), id, "find");
                }
                if ui
                    .add_enabled(
                        dirty && t.saving.is_none(),
                        egui::Button::new(il(icons::SAVE, "Save")).small(),
                    )
                    .on_hover_text("Save (Ctrl+S)")
                    .clicked()
                {
                    send(out, id, EdMsg::Save(tid));
                }
            });
        });
        if ready && dr.bar != Bar::None {
            find_bar(ui, id, t, dr);
        }
        if let Some(err) = &t.error {
            ui.colored_label(red(), format!("{} {err}", icons::X_CIRCLE));
        }
        if t.conflict.is_some() {
            conflict(ui, s, t, out);
        }
    });
    egui::Panel::bottom(("ed-status", id)).show(ui, |ui| status(ui, s, t, dr, out));
    let fill = ui.visuals().extreme_bg_color;
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(fill))
        .show(ui, |ui| match &t.state {
            TabState::Loading => {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Loading…");
                });
            }
            TabState::Failed(why) => {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(format!("{} {why}", icons::WARNING)).color(red()));
                    ui.add_space(8.0);
                    if ui.button("Close tab").clicked() {
                        send(out, id, EdMsg::Close(tid));
                    }
                });
            }
            TabState::Ready => {
                code(ui, id, t, dr, None, None);
            }
        });
}

/// Lines that differ between the two compared texts, kept while the texts
/// stay the same (the LCS is not redone every frame).
type LineMarks = std::sync::Arc<(Vec<bool>, Vec<bool>)>;

fn compare_marks(ctx: &egui::Context, win: u64, left: &str, right: &str) -> LineMarks {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    left.hash(&mut h);
    right.hash(&mut h);
    let hash = h.finish();
    // One entry per window: replaced when the texts change.
    let key = egui::Id::unique(("telepad-ed-compare", win));
    if let Some((k, m)) = ctx.data(|d| d.get_temp::<(u64, LineMarks)>(key))
        && k == hash
    {
        return m;
    }
    let m: LineMarks = std::sync::Arc::new(telepad_core::files::compare::line_marks(left, right));
    ctx.data_mut(|d| d.insert_temp(key, (hash, m.clone())));
    m
}

/// The two files of a Diff (#131) side by side: each editable and saved on
/// its own, differing lines marked.
fn compare_view(
    ui: &mut egui::Ui,
    s: &FileSession,
    a: &EdTab,
    b: &EdTab,
    dr: &mut EdDraft,
    out: &mut Vec<Msg>,
) {
    let id = s.id;
    let ready = a.state == TabState::Ready && b.state == TabState::Ready;
    let marks = if ready {
        let (l, r) = (
            dr.texts.get(&a.id).cloned().unwrap_or_default(),
            dr.texts.get(&b.id).cloned().unwrap_or_default(),
        );
        Some(compare_marks(ui.ctx(), id, &l, &r))
    } else {
        None
    };
    egui::Panel::top(("ed-head", id)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let count = |v: &[bool]| v.iter().filter(|x| **x).count();
            let (deleted, added) = marks
                .as_ref()
                .map_or((0, 0), |m| (count(&m.0), count(&m.1)));
            ui.label(RichText::new(il(icons::NOT_EQUALS, "Compare")).strong());
            if ready {
                if deleted + added == 0 {
                    ui.label("the files are the same");
                } else {
                    let dark = ui.visuals().dark_mode;
                    ui.colored_label(mark_colour(true, dark).1, format!("{added} added"));
                    ui.colored_label(mark_colour(false, dark).1, format!("{deleted} deleted"));
                }
            }
            let mut sync = !dr.no_sync_scroll;
            if ui
                .checkbox(&mut sync, "Sync scroll")
                .on_hover_text("Scrolling one side scrolls the other to the matching line")
                .changed()
            {
                dr.no_sync_scroll = !sync;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button(il(icons::X, "Close compare"))
                    .on_hover_text("Back to one file at a time; both tabs stay open")
                    .clicked()
                {
                    send(out, id, EdMsg::EndCompare);
                }
            });
        });
        for t in [a, b] {
            if let Some(err) = &t.error {
                ui.colored_label(red(), format!("{} {}: {err}", icons::X_CIRCLE, t.name));
            }
            if t.conflict.is_some() {
                conflict(ui, s, t, out);
            }
        }
    });
    // Synchronized scrolling (#135): last offsets and the offsets to apply
    // this frame, per side.
    type Sync = ([egui::Vec2; 2], [Option<egui::Vec2>; 2]);
    let key = egui::Id::unique(("telepad-ed-sync", id));
    let (last, force): Sync = ui.ctx().data(|m| m.get_temp(key)).unwrap_or_default();
    let mut now = [egui::Vec2::ZERO; 2];
    let mut row_h = 14.0f32;
    let fill = ui.visuals().extreme_bg_color;
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(fill))
        .show(ui, |ui| {
            ui.columns(2, |cols| {
                for (k, (t, col)) in [a, b].into_iter().zip(cols.iter_mut()).enumerate() {
                    col.horizontal(|ui| {
                        let icon = match t.side {
                            Side::Local => icons::DESKTOP,
                            Side::Remote => icons::DISKS,
                        };
                        let dirty = dr.dirty(t);
                        if ui
                            .add_enabled(
                                dirty && t.saving.is_none(),
                                egui::Button::new(il(icons::SAVE, "Save")).small(),
                            )
                            .clicked()
                        {
                            send(out, id, EdMsg::Save(t.id));
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("{icon} {}: {}", place(s, t.side), t.path))
                                    .monospace()
                                    .weak(),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&t.path);
                    });
                    match &t.state {
                        TabState::Loading => {
                            col.spinner();
                        }
                        TabState::Failed(why) => {
                            col.colored_label(red(), format!("{} {why}", icons::WARNING));
                        }
                        TabState::Ready => {
                            let m = marks.as_ref().map(|m| Marks {
                                lines: if k == 0 { &m.0 } else { &m.1 },
                                added: k == 1,
                            });
                            let (o, h) = code(col, id, t, dr, m, force[k]);
                            now[k] = o;
                            row_h = h.max(1.0);
                        }
                    }
                }
            });
        });
    let mut next: [Option<egui::Vec2>; 2] = [None, None];
    if !dr.no_sync_scroll
        && let Some(m) = &marks
    {
        let (l2r, r2l) = telepad_core::files::compare::line_map(&m.0, &m.1);
        let moved = |k: usize| force[k].is_none() && (now[k] - last[k]).length() > 0.5;
        // The side the user scrolled leads; the other follows to the matching line.
        let lead = if moved(0) {
            Some((0, &l2r))
        } else if moved(1) {
            Some((1, &r2l))
        } else {
            None
        };
        if let Some((k, map)) = lead {
            let line = (now[k].y / row_h).floor().max(0.0) as usize;
            let frac = now[k].y - line as f32 * row_h;
            let other = map.get(line).copied().unwrap_or(line);
            next[1 - k] = Some(egui::vec2(now[k].x, other as f32 * row_h + frac));
            ui.ctx().request_repaint();
        }
    }
    // A forced side reports the offset it was given, so it does not lead.
    for k in 0..2 {
        if let Some(f) = next[k] {
            now[k] = f;
        }
    }
    ui.ctx()
        .data_mut(|d| d.insert_temp::<Sync>(key, (now, next)));
}

fn focus_field(ctx: &egui::Context, win: u64, which: &'static str) {
    ctx.data_mut(|m| m.insert_temp(egui::Id::unique(("telepad-ed-focus", win)), which));
}

fn take_focus(ctx: &egui::Context, win: u64, which: &'static str) -> bool {
    let k = egui::Id::unique(("telepad-ed-focus", win));
    let want = ctx.data(|m| m.get_temp::<&'static str>(k)) == Some(which);
    if want {
        ctx.data_mut(|m| m.remove::<&'static str>(k));
    }
    want
}

fn tab_button(
    ui: &mut egui::Ui,
    s: &FileSession,
    t: &EdTab,
    active: bool,
    dr: &EdDraft,
    out: &mut Vec<Msg>,
) {
    let id = s.id;
    let icon = match t.side {
        Side::Local => icons::DESKTOP,
        Side::Remote => icons::DISKS,
    };
    let mut label = RichText::new(format!("{icon} {}", t.name));
    if matches!(t.state, TabState::Failed(_)) {
        label = label.color(red());
    }
    let r = ui.selectable_label(active, label).on_hover_text(format!(
        "{}\non {}",
        t.path,
        place(s, t.side)
    ));
    if r.clicked() {
        send(out, id, EdMsg::Activate(t.id));
    }
    if r.middle_clicked() {
        send(out, id, EdMsg::Close(t.id));
    }
    if t.saving.is_some() {
        ui.spinner();
    } else if dr.dirty(t) {
        ui.label(RichText::new("●").strong())
            .on_hover_text("Not saved");
    }
    if ui
        .add(egui::Button::new("×").small().frame(false))
        .on_hover_text("Close (Ctrl+W)")
        .clicked()
    {
        send(out, id, EdMsg::Close(t.id));
    }
    ui.separator();
}

/// Matches of the find text as character ranges (cached while nothing changes).
fn matches(
    ctx: &egui::Context,
    txt: &str,
    query: &str,
    o: FindOpts,
) -> Arc<Result<Vec<Range<usize>>, String>> {
    let mut h = DefaultHasher::new();
    (txt, query, o).hash(&mut h);
    let key = h.finish();
    let slot = egui::Id::unique("telepad-ed-matches");
    type Cached = (u64, Arc<Result<Vec<Range<usize>>, String>>);
    if let Some((k, v)) = ctx.data(|m| m.get_temp::<Cached>(slot))
        && k == key
    {
        return v;
    }
    let v = Arc::new(text::find_all(txt, query, o).map(|r| chars_of(txt, &r)));
    ctx.data_mut(|m| m.insert_temp(slot, (key, v.clone())));
    v
}

/// Byte ranges (in order) → character ranges, in one pass.
fn chars_of(txt: &str, ranges: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut out = Vec::with_capacity(ranges.len());
    let (mut byte, mut ch) = (0usize, 0usize);
    let to = |b: usize, byte: &mut usize, ch: &mut usize| {
        *ch += txt[*byte..b].chars().count();
        *byte = b;
        *ch
    };
    for r in ranges {
        let a = to(r.start, &mut byte, &mut ch);
        let b = to(r.end, &mut byte, &mut ch);
        out.push(a..b);
    }
    out
}

/// Select the next (or previous) match after the cursor.
fn step(
    ctx: &egui::Context,
    win: u64,
    tab: u64,
    dr: &EdDraft,
    forward: bool,
) -> Option<Range<usize>> {
    let txt = dr.texts.get(&tab)?;
    let m = matches(ctx, txt, &dr.find, dr.opts);
    let list = m.as_ref().as_ref().ok()?;
    if list.is_empty() {
        return None;
    }
    let cur = current_in(ctx, win, tab, list)
        .or_else(|| cursor_of(ctx, win, tab))
        .unwrap_or(0..0);
    let r = if forward {
        list.iter()
            .find(|r| r.start >= cur.end && *r != &cur)
            .or(list.first())
    } else {
        list.iter()
            .rev()
            .find(|r| r.end <= cur.start)
            .or(list.last())
    }?
    .clone();
    select_later(ctx, win, tab, r.clone(), false);
    set_current(ctx, win, tab, Some(r.clone()));
    Some(r)
}

fn toggle(ui: &mut egui::Ui, on: &mut bool, text: &str, hint: &str) {
    if ui
        .selectable_label(*on, RichText::new(text).monospace())
        .on_hover_text(hint)
        .clicked()
    {
        *on = !*on;
    }
}

fn find_bar(ui: &mut egui::Ui, win: u64, t: &EdTab, dr: &mut EdDraft) {
    let tab = t.id;
    let ctx = ui.ctx().clone();
    if dr.bar == Bar::Goto {
        let n = dr.texts.get(&tab).map_or(1, |x| text::line_count(x));
        ui.horizontal(|ui| {
            ui.label(format!("Go to line (1–{n}):"));
            let r = ui.add(egui::TextEdit::singleline(&mut dr.goto).desired_width(90.0));
            if take_focus(&ctx, win, "goto") {
                r.request_focus();
            }
            // Taken here: the text gets the focus in this same frame and would get a newline.
            let enter =
                r.lost_focus() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
            if (enter || ui.button("Go").clicked())
                && let Ok(line) = dr.goto.trim().parse::<usize>()
                && let Some(txt) = dr.texts.get(&tab)
            {
                let c = text::line_start(txt, line.max(1));
                select_later(&ctx, win, tab, c..c, true);
                dr.bar = Bar::None;
            }
            if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
                dr.bar = Bar::None;
            }
        });
        return;
    }
    let mut replaced = None;
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut dr.find)
                .hint_text("Find")
                .desired_width(260.0)
                .font(egui::TextStyle::Monospace),
        );
        if take_focus(&ctx, win, "find") {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let back = ui.input(|i| i.modifiers.shift);
            step(&ctx, win, tab, dr, !back);
            r.request_focus();
        }
        toggle(ui, &mut dr.opts.case, "Aa", "Match case");
        toggle(ui, &mut dr.opts.whole, "ab", "Whole word");
        toggle(
            ui,
            &mut dr.opts.regex,
            ".*",
            "Regular expression ($1 in the replacement)",
        );
        let txt = dr.texts.get(&tab).cloned().unwrap_or_default();
        let m = matches(&ctx, &txt, &dr.find, dr.opts);
        match m.as_ref() {
            Err(e) => {
                ui.colored_label(red(), e);
            }
            Ok(list) if !dr.find.is_empty() => {
                let cur = current_in(&ctx, win, tab, list);
                let at = cur.and_then(|c| list.iter().position(|r| *r == c));
                let text = match (at, list.len()) {
                    (_, 0) => "No results".to_string(),
                    (Some(i), n) => format!("{} of {n}", i + 1),
                    (None, n) => format!("{n} found"),
                };
                ui.label(RichText::new(text).weak());
            }
            Ok(_) => {}
        }
        if ui
            .small_button(icons::CARET_UP)
            .on_hover_text("Previous (Shift+Enter, Shift+F3)")
            .clicked()
        {
            step(&ctx, win, tab, dr, false);
        }
        if ui
            .small_button(icons::CARET_DOWN)
            .on_hover_text("Next (Enter, F3)")
            .clicked()
        {
            step(&ctx, win, tab, dr, true);
        }
        if dr.bar == Bar::Find
            && ui
                .small_button(il(icons::SYNC, "Replace"))
                .on_hover_text("Ctrl+H")
                .clicked()
        {
            dr.bar = Bar::Replace;
        }
        if ui.small_button("×").on_hover_text("Close (Esc)").clicked() {
            dr.bar = Bar::None;
        }
    });
    if dr.bar == Bar::Replace {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut dr.replace)
                    .hint_text("Replace with")
                    .desired_width(260.0)
                    .font(egui::TextStyle::Monospace),
            );
            if ui
                .button("Replace")
                .on_hover_text("Replace this match, go to the next")
                .clicked()
            {
                replaced = Some(false);
            }
            if ui.button("Replace all").clicked() {
                replaced = Some(true);
            }
        });
    }
    let Some(all) = replaced else {
        return;
    };
    let Some(txt) = dr.texts.get(&tab).cloned() else {
        return;
    };
    if all {
        if let Ok((new, n)) = text::replace_all(&txt, &dr.find, &dr.replace, dr.opts)
            && n > 0
        {
            dr.texts.insert(tab, new);
            ctx.data_mut(|m| m.insert_temp(egui::Id::unique(("telepad-ed-replaced", win)), n));
        }
        return;
    }
    // Replace the current match (if there is one), then go on to the next.
    let m = matches(&ctx, &txt, &dr.find, dr.opts);
    let cur = m
        .as_ref()
        .as_ref()
        .ok()
        .and_then(|l| current_in(&ctx, win, tab, l));
    if let Some(cur) = cur {
        let at = text::char_to_byte(&txt, cur.start);
        if let Ok(Some((new, r))) = text::replace_at(&txt, at, &dr.find, &dr.replace, dr.opts) {
            // Select the next match after the replacement, ready for the next click.
            let end = text::byte_to_char(&new, r.end);
            let m = matches(&ctx, &new, &dr.find, dr.opts);
            let next = m
                .as_ref()
                .as_ref()
                .ok()
                .and_then(|l| l.iter().find(|x| x.start >= end).or(l.first()).cloned());
            dr.texts.insert(tab, new);
            select_later(&ctx, win, tab, next.clone().unwrap_or(end..end), false);
            set_current(&ctx, win, tab, next);
        }
    } else {
        step(&ctx, win, tab, dr, true);
    }
}

fn conflict(ui: &mut egui::Ui, s: &FileSession, t: &EdTab, out: &mut Vec<Msg>) {
    let id = s.id;
    let Some(c) = &t.conflict else {
        return;
    };
    let warn = usage_color(80.0);
    egui::Frame::group(ui.style())
        .stroke(egui::Stroke::new(1.0, warn))
        .show(ui, |ui| {
            let now = match c.now {
                Some(st) => format!(
                    "it is now {}, modified {} UTC",
                    human_size(st.size),
                    time_text(st.mtime)
                ),
                None => "it was deleted".into(),
            };
            ui.label(
                RichText::new(format!(
                    "{} {} changed on {} after you opened it: {now}.",
                    icons::WARNING,
                    t.name,
                    place(s, t.side)
                ))
                .color(warn),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(egui::Button::new(act_text(Act::Danger, "Overwrite")))
                    .on_hover_text("Save your text over the file as it is now")
                    .clicked()
                {
                    send(out, id, EdMsg::Overwrite(t.id));
                }
                if ui
                    .button("Reload")
                    .on_hover_text("Discard your changes and load the file as it is now")
                    .clicked()
                {
                    send(out, id, EdMsg::Reload(t.id));
                }
                if ui
                    .add_enabled(
                        !c.diff_loading && c.now.is_some(),
                        egui::Button::new("Show diff"),
                    )
                    .clicked()
                {
                    send(out, id, EdMsg::ShowDiff(t.id));
                }
                if ui
                    .button("Keep editing")
                    .on_hover_text("Close this question; Save asks again")
                    .clicked()
                {
                    send(out, id, EdMsg::KeepEditing(t.id));
                }
                if c.diff_loading {
                    ui.spinner();
                }
            });
            match &c.diff {
                Some(Err(e)) => {
                    ui.colored_label(red(), e);
                }
                Some(Ok(lines)) => diff_view(ui, id, t.id, lines),
                None => {}
            }
        });
}

/// Changed lines with two lines around them; longer unchanged runs fold.
fn diff_view(ui: &mut egui::Ui, win: u64, tab: u64, lines: &[(char, String)]) {
    let near: Vec<bool> = (0..lines.len())
        .map(|i| {
            let lo = i.saturating_sub(2);
            let hi = (i + 3).min(lines.len());
            lines[lo..hi].iter().any(|(k, _)| *k != ' ')
        })
        .collect();
    ui.label(
        RichText::new("− in the file now    + your text")
            .weak()
            .small(),
    );
    egui::ScrollArea::vertical()
        .id_salt(("ed-diff", win, tab))
        .max_height(240.0)
        .min_scrolled_height(160.0)
        .show(ui, |ui| {
            let mut skipped = 0;
            for (i, (k, l)) in lines.iter().enumerate() {
                if !near[i] {
                    skipped += 1;
                    continue;
                }
                if skipped > 0 {
                    ui.label(
                        RichText::new(format!("  … {skipped} unchanged line(s)"))
                            .weak()
                            .small(),
                    );
                    skipped = 0;
                }
                let t = RichText::new(format!("{k} {l}")).monospace();
                ui.label(match k {
                    '-' => t.color(red()),
                    '+' => t.color(usage_color(10.0)),
                    _ => t.weak(),
                });
            }
            if skipped > 0 {
                ui.label(
                    RichText::new(format!("  … {skipped} unchanged line(s)"))
                        .weak()
                        .small(),
                );
            }
            if lines.iter().all(|(k, _)| *k == ' ') {
                ui.label("No differences in the text (only the time or line endings changed).");
            }
        });
}

fn status(ui: &mut egui::Ui, s: &FileSession, t: &EdTab, dr: &EdDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    ui.horizontal(|ui| {
        let state = if t.saving.is_some() {
            "Saving…".to_string()
        } else if dr.dirty(t) {
            "Not saved".to_string()
        } else if t.state == TabState::Ready {
            "Saved".to_string()
        } else {
            String::new()
        };
        ui.label(RichText::new(state).weak());
        let replaced = egui::Id::unique(("telepad-ed-replaced", id));
        if dr.bar != Bar::Replace {
            ui.ctx().data_mut(|m| m.remove::<usize>(replaced));
        }
        if let Some(n) = ui.ctx().data_mut(|m| m.get_temp::<usize>(replaced)) {
            ui.label(RichText::new(format!("{n} replaced")).weak());
        }
        if t.state != TabState::Ready {
            return;
        }
        let txt = dr.texts.get(&t.id).map(String::as_str).unwrap_or("");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(if t.bom { "UTF-8 with BOM" } else { "UTF-8" });
            ui.separator();
            let eol = ui.label(if t.crlf { "CRLF" } else { "LF" });
            if t.mixed {
                eol.on_hover_text(format!(
                    "The file had both line endings; it is saved with {}",
                    if t.crlf { "CRLF" } else { "LF" }
                ));
            }
            ui.separator();
            egui::ComboBox::from_id_salt(("ed-lang", id, t.id))
                .selected_text(t.lang.name())
                .show_ui(ui, |ui| {
                    for l in Lang::ALL {
                        if ui.selectable_label(l == t.lang, l.name()).clicked() {
                            send(out, id, EdMsg::SetLang(t.id, l));
                        }
                    }
                });
            ui.separator();
            let unit = text::indent_unit(txt);
            ui.label(if unit == "\t" {
                "Tab".to_string()
            } else {
                format!("Spaces: {}", unit.len())
            });
            ui.separator();
            let cur = cursor_of(ui.ctx(), id, t.id).unwrap_or(0..0);
            let (l, c) = text::line_col(txt, cur.end);
            let sel = if cur.is_empty() {
                String::new()
            } else {
                format!(" ({} selected)", cur.len())
            };
            ui.label(format!("Ln {l}, Col {c}{sel}"));
        });
    });
}

fn colour(t: Tok, dark: bool, plain: Color32) -> Color32 {
    let rgb = |v: u32| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
    // VS Code's Dark+ / Light+ colours.
    match (t, dark) {
        (Tok::Plain, _) => plain,
        (Tok::Comment, true) => rgb(0x6A9955),
        (Tok::Comment, false) => rgb(0x008000),
        (Tok::Str, true) => rgb(0xCE9178),
        (Tok::Str, false) => rgb(0xA31515),
        (Tok::Num, true) => rgb(0xB5CEA8),
        (Tok::Num, false) => rgb(0x098658),
        (Tok::Keyword, true) => rgb(0x569CD6),
        (Tok::Keyword, false) => rgb(0x0000FF),
        (Tok::Key, true) => rgb(0x9CDCFE),
        (Tok::Key, false) => rgb(0x0451A5),
        (Tok::Heading, true) => rgb(0x4EC9B0),
        (Tok::Heading, false) => rgb(0x800000),
        (Tok::Var, true) => rgb(0x4FC1FF),
        (Tok::Var, false) => rgb(0x001080),
    }
}

/// Coloured layout of the text (spans cached while the text is unchanged).
fn layout(ui: &egui::Ui, txt: &str, lang: Lang, font: &egui::FontId) -> LayoutJob {
    let dark = ui.visuals().dark_mode;
    let plain = ui.visuals().text_color();
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    if txt.len() > COLOUR_LIMIT || lang == Lang::Plain {
        job.append(txt, 0.0, TextFormat::simple(font.clone(), plain));
        return job;
    }
    let mut h = DefaultHasher::new();
    (txt, lang).hash(&mut h);
    let key = h.finish();
    let slot = egui::Id::unique("telepad-ed-spans");
    type Cached = (u64, Arc<Vec<(Range<usize>, Tok)>>);
    let spans = match ui.ctx().data(|m| m.get_temp::<Cached>(slot)) {
        Some((k, v)) if k == key => v,
        _ => {
            let v = Arc::new(syntax::highlight(lang, txt));
            ui.ctx().data_mut(|m| m.insert_temp(slot, (key, v.clone())));
            v
        }
    };
    for (r, tok) in spans.iter() {
        job.append(
            &txt[r.clone()],
            0.0,
            TextFormat::simple(font.clone(), colour(*tok, dark, plain)),
        );
    }
    job
}

/// Lines of one side of a compare (#131, #135): which lines, and whether
/// they are added (green `+`, right side) or deleted (red `-`, left side).
#[derive(Clone, Copy)]
struct Marks<'a> {
    lines: &'a [bool],
    added: bool,
}

fn mark_colour(added: bool, dark: bool) -> (Color32, Color32) {
    // (line background, sign)
    match (added, dark) {
        (true, true) => (
            Color32::from_rgba_unmultiplied(46, 160, 67, 55),
            Color32::from_rgb(87, 200, 110),
        ),
        (true, false) => (
            Color32::from_rgba_unmultiplied(46, 160, 67, 60),
            Color32::from_rgb(26, 127, 55),
        ),
        (false, true) => (
            Color32::from_rgba_unmultiplied(248, 81, 73, 55),
            Color32::from_rgb(255, 123, 114),
        ),
        (false, false) => (
            Color32::from_rgba_unmultiplied(248, 81, 73, 55),
            Color32::from_rgb(207, 34, 46),
        ),
    }
}

/// `marks`: the differing lines of a compare; `scroll_to`: vertical offset
/// to show (synchronized scrolling, #135). Returns (vertical offset, row
/// height) after drawing.
fn code(
    ui: &mut egui::Ui,
    win: u64,
    t: &EdTab,
    dr: &mut EdDraft,
    marks: Option<Marks>,
    scroll_to: Option<egui::Vec2>,
) -> (egui::Vec2, f32) {
    let eid = edit_id(win, t.id);
    let ctx = ui.ctx().clone();
    let Some(buf) = dr.texts.get_mut(&t.id) else {
        return (egui::Vec2::ZERO, 14.0);
    };
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    // Tab / Shift+Tab: indent / outdent the selected lines.
    if ui.memory(|m| m.has_focus(eid)) {
        let out = ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab));
        let inn = !out && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab));
        if (out || inn)
            && let Some(mut st) = egui::TextEdit::load_state(&ctx, eid)
            && let Some(r) = st.cursor.char_range()
        {
            let (a, b) = (r.primary.index.0, r.secondary.index.0);
            let unit = text::indent_unit(buf);
            let (new, sel) = text::indent(buf, a.min(b)..a.max(b), &unit, out);
            *buf = new;
            st.cursor.set_char_range(Some(CCursorRange::two(
                CCursor::new(sel.start),
                CCursor::new(sel.end),
            )));
            st.store(&ctx, eid);
        }
    }
    // A selection asked for by find / go to line.
    let pending = ctx.data_mut(|m| m.remove_temp::<(Range<usize>, bool)>(eid.with("select")));
    if let Some((r, focus)) = &pending {
        let mut st = egui::TextEdit::load_state(&ctx, eid).unwrap_or_default();
        st.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(r.start),
            CCursor::new(r.end),
        )));
        st.store(&ctx, eid);
        if *focus {
            ctx.memory_mut(|m| m.request_focus(eid));
        }
    }
    let lines = text::line_count(buf);
    let digits = lines.to_string().len().max(3);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&mono, '0'));
    let gutter = digits as f32 * char_w + 24.0;
    let lang = t.lang;
    let find = (dr.bar != Bar::None && dr.bar != Bar::Goto && !dr.find.is_empty())
        .then(|| (dr.find.clone(), dr.opts));
    let mut layouter = |ui: &egui::Ui, b: &dyn egui::TextBuffer, _wrap: f32| {
        let job = layout(ui, b.as_str(), lang, &mono);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut area = egui::ScrollArea::both()
        .id_salt(("ed-scroll", win, t.id))
        .auto_shrink([false, false]);
    if let Some(o) = scroll_to {
        area = area
            .vertical_scroll_offset(o.y)
            .horizontal_scroll_offset(o.x);
    }
    let mut row_height = 14.0;
    let shown = area.show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.add_space(gutter);
            let under = ui.painter().add(egui::Shape::Noop);
            let min = egui::vec2(ui.available_width(), ui.available_height().max(100.0));
            let output = egui::TextEdit::multiline(buf)
                .id(eid)
                .code_editor()
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::symmetric(4, 2))
                .desired_width(f32::INFINITY)
                .min_size(min)
                .layouter(&mut layouter)
                .show(ui);
            let galley = &output.galley;
            let origin = output.galley_pos;
            let clip = ui.clip_rect();
            let row_h = galley
                .rows
                .first()
                .map_or(14.0, |r| r.rect().height())
                .max(1.0);
            let cursor = output
                .cursor_range
                .or_else(|| output.state.cursor.char_range());
            let cur_rect = cursor.map(|c| {
                galley
                    .pos_from_cursor(c.primary)
                    .translate(origin.to_vec2())
            });
            let mut shapes = Vec::new();
            let v = ui.visuals();
            row_height = row_h;
            if let Some(mk) = marks {
                let (tint, _) = mark_colour(mk.added, v.dark_mode);
                for (i, row) in galley.rows.iter().enumerate() {
                    if !mk.lines.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    let y0 = origin.y + row.rect().min.y;
                    let y1 = origin.y + row.rect().max.y;
                    if y1 < clip.top() || y0 > clip.bottom() {
                        continue;
                    }
                    shapes.push(egui::Shape::rect_filled(
                        egui::Rect::from_x_y_ranges(clip.x_range(), y0..=y1),
                        0.0,
                        tint,
                    ));
                }
            }
            if let Some(r) = cur_rect {
                let line = egui::Rect::from_x_y_ranges(clip.x_range(), r.y_range());
                shapes.push(egui::Shape::rect_filled(
                    line,
                    0.0,
                    if v.dark_mode {
                        Color32::from_white_alpha(12)
                    } else {
                        Color32::from_black_alpha(14)
                    },
                ));
            }
            if let Some((q, o)) = &find {
                let m = matches(&ctx, buf.as_str(), q, *o);
                if let Ok(list) = m.as_ref() {
                    let sel = current(&ctx, win, t.id);
                    let base = v.selection.bg_fill;
                    for r in list {
                        let a = galley
                            .pos_from_cursor(CCursor::new(r.start))
                            .translate(origin.to_vec2());
                        if a.bottom() < clip.top() || a.top() > clip.bottom() {
                            continue;
                        }
                        let b = galley
                            .pos_from_cursor(CCursor::new(r.end))
                            .translate(origin.to_vec2());
                        let rect = if (b.top() - a.top()).abs() < 1.0 {
                            egui::Rect::from_min_max(
                                a.min,
                                egui::pos2(b.max.x.max(a.min.x + 2.0), a.max.y),
                            )
                        } else {
                            egui::Rect::from_min_max(a.min, egui::pos2(clip.right(), a.max.y))
                        };
                        let strong = sel.as_ref() == Some(r);
                        let fill = if strong {
                            base
                        } else {
                            base.gamma_multiply(0.45)
                        };
                        shapes.push(egui::Shape::rect_filled(rect, 2.0, fill));
                    }
                }
            }
            ui.painter().set(under, egui::Shape::Vec(shapes));
            // Line numbers stay at the left edge when scrolled sideways.
            let gx = clip.left();
            let gut = egui::Rect::from_min_max(
                egui::pos2(gx, clip.top()),
                egui::pos2(gx + gutter, clip.bottom()),
            );
            let p = ui.painter();
            p.rect_filled(gut, 0.0, v.extreme_bg_color);
            let cur_line = cur_rect.map(|r| ((r.center().y - origin.y) / row_h).floor() as usize);
            let first = (((clip.top() - origin.y) / row_h).floor().max(0.0)) as usize;
            let last = ((((clip.bottom() - origin.y) / row_h).ceil()).max(0.0) as usize)
                .min(galley.rows.len());
            for (i, row) in galley.rows.iter().enumerate().take(last).skip(first) {
                let y = origin.y + row.rect().min.y;
                let colour = if Some(i) == cur_line {
                    v.strong_text_color()
                } else {
                    v.weak_text_color()
                };
                p.text(
                    egui::pos2(gx + gutter - 12.0, y),
                    egui::Align2::RIGHT_TOP,
                    (i + 1).to_string(),
                    mono.clone(),
                    colour,
                );
                // git-style sign of a compared line (#135): - deleted, + added.
                if let Some(mk) = marks
                    && mk.lines.get(i).copied().unwrap_or(false)
                {
                    let (_, sign) = mark_colour(mk.added, v.dark_mode);
                    p.text(
                        egui::pos2(gx + gutter - 4.0, y),
                        egui::Align2::RIGHT_TOP,
                        if mk.added { "+" } else { "-" },
                        mono.clone(),
                        sign,
                    );
                }
            }
            if pending.is_some()
                && let Some(r) = cur_rect
            {
                ui.scroll_to_rect(
                    r.expand2(egui::vec2(gutter + 40.0, row_h * 2.0)),
                    Some(egui::Align::Center),
                );
            }
        });
    });
    (shown.state.clamped_offset(), row_height)
}

/// Save / Don't save / Cancel for unsaved tabs.
fn question(ui: &mut egui::Ui, s: &FileSession, dr: &EdDraft, out: &mut Vec<Msg>) {
    let id = s.id;
    let e = &s.editor;
    let Some(q) = e.ask else {
        return;
    };
    let names: Vec<String> = match q {
        Unsaved::Tab(t) => e.tab(t).map(|t| t.name.clone()).into_iter().collect(),
        Unsaved::All { .. } => e
            .tabs
            .iter()
            .filter(|t| dr.dirty(t))
            .map(|t| t.name.clone())
            .collect(),
    };
    let modal = egui::Modal::new(egui::Id::unique(("telepad-ed-ask", id))).show(ui.ctx(), |ui| {
        ui.set_width(440.0);
        ui.heading(il(icons::WARNING, "Save changes?"));
        match q {
            Unsaved::Tab(_) => {
                ui.label(format!(
                    "{} has changes that are not saved.",
                    names.join(", ")
                ));
            }
            Unsaved::All { window } => {
                ui.label(if window {
                    format!(
                        "The file window of {} closes. {} file(s) have changes that are not saved:",
                        s.host.alias,
                        names.len()
                    )
                } else {
                    format!("{} file(s) have changes that are not saved:", names.len())
                });
                for n in &names {
                    ui.label(RichText::new(format!("  {n}")).monospace());
                }
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button(il(icons::SAVE, "Save")).clicked() {
                send(out, id, EdMsg::Answer(Answer::Save));
            }
            if ui
                .add(egui::Button::new(act_text(Act::Danger, "Don't save")))
                .clicked()
            {
                send(out, id, EdMsg::Answer(Answer::Discard));
            }
            if ui.button("Cancel").clicked() {
                send(out, id, EdMsg::Answer(Answer::Cancel));
            }
        });
    });
    if modal.should_close() {
        send(out, id, EdMsg::Answer(Answer::Cancel));
    }
}

/// Telepad's text editor for a text that is not a file of a file window
/// (#173: a Portainer stack's compose file): the same code area, colours,
/// line numbers, find / replace / go to line (Ctrl+F / H / G, F3) and
/// indenting as the editor window. `win` keeps its state apart from other
/// windows; `editable` = false shows the text read-only (edits are dropped).
#[allow(clippy::too_many_arguments)]
pub fn embedded(
    ui: &mut egui::Ui,
    win: u64,
    focused: bool,
    name: &str,
    lang: Lang,
    text: &mut String,
    saved: &str,
    editable: bool,
) {
    const TAB: u64 = 1;
    let slot = egui::Id::unique(("telepad-embedded-ed", win));
    let mut dr: EdDraft = ui.data(|m| m.get_temp(slot)).unwrap_or_default();
    let before = text.clone();
    dr.texts.insert(TAB, std::mem::take(text));
    let t = EdTab {
        id: TAB,
        side: Side::Remote,
        path: name.to_string(),
        name: name.to_string(),
        lang,
        state: TabState::Ready,
        crlf: false,
        mixed: false,
        bom: false,
        stamp: None,
        saved: saved.to_string(),
        saving: None,
        error: None,
        conflict: None,
        close_after_save: false,
    };
    if focused {
        let sc = |m: Modifiers, k: Key| {
            ui.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k)))
        };
        let find = sc(Modifiers::COMMAND, Key::F);
        let replace = editable && sc(Modifiers::COMMAND, Key::H);
        if find || replace {
            dr.bar = if replace { Bar::Replace } else { Bar::Find };
            if let (Some(r), Some(txt)) = (cursor_of(ui.ctx(), win, TAB), dr.texts.get(&TAB))
                && !r.is_empty()
            {
                let (a, b) = (
                    text::char_to_byte(txt, r.start),
                    text::char_to_byte(txt, r.end),
                );
                if !txt[a..b].contains('\n') {
                    dr.find = txt[a..b].to_string();
                }
            }
            focus_field(ui.ctx(), win, "find");
        }
        if sc(Modifiers::COMMAND, Key::G) {
            dr.bar = Bar::Goto;
            dr.goto.clear();
            focus_field(ui.ctx(), win, "goto");
        }
        if dr.bar != Bar::None && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            dr.bar = Bar::None;
            match current(ui.ctx(), win, TAB) {
                Some(r) => select_later(ui.ctx(), win, TAB, r, true),
                None => ui.memory_mut(|m| m.request_focus(edit_id(win, TAB))),
            }
            set_current(ui.ctx(), win, TAB, None);
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F3)) {
            step(ui.ctx(), win, TAB, &dr, true);
        }
        if ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F3)) {
            step(ui.ctx(), win, TAB, &dr, false);
        }
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(il(icons::CODE, name)).monospace().weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(lang.name()).weak());
            if !editable {
                ui.label(RichText::new("read-only").weak());
            }
            if ui
                .add(egui::Button::new(il(icons::SEARCH, "Find")).small())
                .on_hover_text("Find (Ctrl+F), replace (Ctrl+H), go to line (Ctrl+G)")
                .clicked()
            {
                dr.bar = Bar::Find;
                focus_field(ui.ctx(), win, "find");
            }
        });
    });
    if dr.bar == Bar::Replace && !editable {
        dr.bar = Bar::Find;
    }
    if dr.bar != Bar::None {
        find_bar(ui, win, &t, &mut dr);
    }
    let fill = ui.visuals().extreme_bg_color;
    egui::Frame::new().fill(fill).show(ui, |ui| {
        code(ui, win, &t, &mut dr, None, None);
    });
    *text = dr.texts.remove(&TAB).unwrap_or_default();
    if !editable {
        *text = before;
    }
    ui.data_mut(|m| m.insert_temp(slot, dr));
}
