//! Editor keyboard shortcut profiles (#205): VS Code (default), Sublime Text,
//! JetBrains, Notepad++, Emacs and nano. A profile maps each [`Cmd`] to one
//! or more key sequences written as text ("Ctrl+S", "Ctrl+X Ctrl+S"); the
//! [`Matcher`] takes them out of a frame's input before the text field sees
//! them. Keys a profile leaves to the text field itself (Ctrl+Z, Ctrl+A,
//! Ctrl+V, Home, End in most profiles) are not taken: [`NATIVE`].
//! Modal styles (Vim, Helix) are out of scope (owner decision on #205).

use eframe::egui::{self, Event, Key, Modifiers};

/// What a shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cmd {
    Save,
    SaveAs,
    Open,
    CloseTab,
    Find,
    Replace,
    FindNext,
    /// Previous match / search backward.
    FindPrev,
    GotoLine,
    MatchingBracket,
    CopyAll,
    /// Cut the selection, or the current line when nothing is selected.
    CutLine,
    /// Copy the selection, or the current line.
    CopyLine,
    Paste,
    DeleteLine,
    Undo,
    Redo,
    LineStart,
    LineEnd,
    SelectAll,
    /// Set / unset the mark: moving the cursor selects (Emacs, nano).
    Mark,
    InsertFile,
    Help,
}

impl Cmd {
    pub const ALL: [Cmd; 23] = [
        Cmd::Save,
        Cmd::SaveAs,
        Cmd::Open,
        Cmd::InsertFile,
        Cmd::CloseTab,
        Cmd::Find,
        Cmd::Replace,
        Cmd::FindNext,
        Cmd::FindPrev,
        Cmd::GotoLine,
        Cmd::MatchingBracket,
        Cmd::CutLine,
        Cmd::CopyLine,
        Cmd::Paste,
        Cmd::CopyAll,
        Cmd::DeleteLine,
        Cmd::Undo,
        Cmd::Redo,
        Cmd::SelectAll,
        Cmd::Mark,
        Cmd::LineStart,
        Cmd::LineEnd,
        Cmd::Help,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Cmd::Save => "Save",
            Cmd::SaveAs => "Save as / write out",
            Cmd::Open => "Open another file",
            Cmd::CloseTab => "Close tab",
            Cmd::Find => "Find",
            Cmd::Replace => "Replace",
            Cmd::FindNext => "Find next",
            Cmd::FindPrev => "Find previous / search backward",
            Cmd::GotoLine => "Go to line",
            Cmd::MatchingBracket => "Go to matching bracket",
            Cmd::CopyAll => "Copy all",
            Cmd::CutLine => "Cut line / selection",
            Cmd::CopyLine => "Copy line / selection",
            Cmd::Paste => "Paste",
            Cmd::DeleteLine => "Delete line",
            Cmd::Undo => "Undo",
            Cmd::Redo => "Redo",
            Cmd::LineStart => "Start of line",
            Cmd::LineEnd => "End of line",
            Cmd::SelectAll => "Select all",
            Cmd::Mark => "Set / unset mark (select by moving)",
            Cmd::InsertFile => "Insert a file",
            Cmd::Help => "Keyboard shortcuts (this list)",
        }
    }

    /// Works on the text, so only while the text has the keyboard focus.
    pub fn on_text(self) -> bool {
        matches!(
            self,
            Cmd::CutLine
                | Cmd::CopyLine
                | Cmd::Paste
                | Cmd::CopyAll
                | Cmd::DeleteLine
                | Cmd::Undo
                | Cmd::Redo
                | Cmd::LineStart
                | Cmd::LineEnd
                | Cmd::SelectAll
                | Cmd::Mark
                | Cmd::MatchingBracket
        )
    }
}

/// A profile: its key (saved in `settings` as `ui_editor_keymap`), name and
/// bindings. A command may have several sequences; `"*"` after a sequence
/// marks a key Telepad chose because that editor has none by default.
pub struct Profile {
    pub key: &'static str,
    pub name: &'static str,
    pub binds: &'static [(Cmd, &'static [&'static str])],
}

/// Keys the text field handles itself: left to it when a profile uses them.
pub const NATIVE: [(Cmd, &str); 7] = [
    (Cmd::Undo, "Ctrl+Z"),
    (Cmd::Redo, "Ctrl+Y"),
    (Cmd::Redo, "Ctrl+Shift+Z"),
    (Cmd::SelectAll, "Ctrl+A"),
    (Cmd::Paste, "Ctrl+V"),
    (Cmd::LineStart, "Home"),
    (Cmd::LineEnd, "End"),
];

pub const PROFILES: [Profile; 6] = [
    Profile {
        key: "vscode",
        name: "VS Code",
        binds: &[
            (Cmd::Save, &["Ctrl+S"]),
            (Cmd::SaveAs, &["Ctrl+Shift+S"]),
            (Cmd::Open, &["Ctrl+O"]),
            (Cmd::InsertFile, &["Ctrl+Shift+I*"]),
            (Cmd::CloseTab, &["Ctrl+W", "Ctrl+F4"]),
            (Cmd::Find, &["Ctrl+F"]),
            (Cmd::Replace, &["Ctrl+H"]),
            (Cmd::FindNext, &["F3"]),
            (Cmd::FindPrev, &["Shift+F3"]),
            (Cmd::GotoLine, &["Ctrl+G"]),
            (Cmd::MatchingBracket, &["Ctrl+Shift+Backslash"]),
            (Cmd::CutLine, &["Ctrl+X"]),
            (Cmd::CopyLine, &["Ctrl+C"]),
            (Cmd::Paste, &["Ctrl+V"]),
            (Cmd::CopyAll, &["Ctrl+Shift+C*"]),
            (Cmd::DeleteLine, &["Ctrl+Shift+K"]),
            (Cmd::Undo, &["Ctrl+Z"]),
            (Cmd::Redo, &["Ctrl+Y", "Ctrl+Shift+Z"]),
            (Cmd::SelectAll, &["Ctrl+A"]),
            (Cmd::LineStart, &["Home"]),
            (Cmd::LineEnd, &["End"]),
            (Cmd::Help, &["Ctrl+K Ctrl+S"]),
        ],
    },
    Profile {
        key: "sublime",
        name: "Sublime Text",
        binds: &[
            (Cmd::Save, &["Ctrl+S"]),
            (Cmd::SaveAs, &["Ctrl+Shift+S"]),
            (Cmd::Open, &["Ctrl+O"]),
            (Cmd::InsertFile, &["Ctrl+Shift+I*"]),
            (Cmd::CloseTab, &["Ctrl+W", "Ctrl+F4"]),
            (Cmd::Find, &["Ctrl+F"]),
            (Cmd::Replace, &["Ctrl+H"]),
            (Cmd::FindNext, &["F3"]),
            (Cmd::FindPrev, &["Shift+F3"]),
            (Cmd::GotoLine, &["Ctrl+G"]),
            (Cmd::MatchingBracket, &["Ctrl+M"]),
            (Cmd::CutLine, &["Ctrl+X"]),
            (Cmd::CopyLine, &["Ctrl+C"]),
            (Cmd::Paste, &["Ctrl+V"]),
            (Cmd::CopyAll, &["Ctrl+Shift+C*"]),
            (Cmd::DeleteLine, &["Ctrl+Shift+K"]),
            (Cmd::Undo, &["Ctrl+Z"]),
            (Cmd::Redo, &["Ctrl+Y", "Ctrl+Shift+Z"]),
            (Cmd::SelectAll, &["Ctrl+A"]),
            (Cmd::LineStart, &["Home"]),
            (Cmd::LineEnd, &["End"]),
            (Cmd::Help, &["F1*"]),
        ],
    },
    Profile {
        key: "jetbrains",
        name: "JetBrains",
        binds: &[
            (Cmd::Save, &["Ctrl+S"]),
            (Cmd::SaveAs, &["Ctrl+Shift+S*"]),
            (Cmd::Open, &["Ctrl+Shift+N"]),
            (Cmd::InsertFile, &["Ctrl+Shift+I*"]),
            (Cmd::CloseTab, &["Ctrl+F4"]),
            (Cmd::Find, &["Ctrl+F"]),
            (Cmd::Replace, &["Ctrl+R"]),
            (Cmd::FindNext, &["F3"]),
            (Cmd::FindPrev, &["Shift+F3"]),
            (Cmd::GotoLine, &["Ctrl+G"]),
            (Cmd::MatchingBracket, &["Ctrl+Shift+M"]),
            (Cmd::CutLine, &["Ctrl+X"]),
            (Cmd::CopyLine, &["Ctrl+C"]),
            (Cmd::Paste, &["Ctrl+V"]),
            (Cmd::CopyAll, &["Ctrl+Shift+C*"]),
            (Cmd::DeleteLine, &["Ctrl+Y"]),
            (Cmd::Undo, &["Ctrl+Z"]),
            (Cmd::Redo, &["Ctrl+Shift+Z"]),
            (Cmd::SelectAll, &["Ctrl+A"]),
            (Cmd::LineStart, &["Home"]),
            (Cmd::LineEnd, &["End"]),
            (Cmd::Help, &["F1*"]),
        ],
    },
    Profile {
        key: "notepadpp",
        name: "Notepad++",
        binds: &[
            (Cmd::Save, &["Ctrl+S"]),
            (Cmd::SaveAs, &["Ctrl+Alt+S"]),
            (Cmd::Open, &["Ctrl+O"]),
            (Cmd::InsertFile, &["Ctrl+Shift+I*"]),
            (Cmd::CloseTab, &["Ctrl+W"]),
            (Cmd::Find, &["Ctrl+F"]),
            (Cmd::Replace, &["Ctrl+H"]),
            (Cmd::FindNext, &["F3"]),
            (Cmd::FindPrev, &["Shift+F3"]),
            (Cmd::GotoLine, &["Ctrl+G"]),
            (Cmd::MatchingBracket, &["Ctrl+B"]),
            (Cmd::CutLine, &["Ctrl+X", "Ctrl+L"]),
            (Cmd::CopyLine, &["Ctrl+C"]),
            (Cmd::Paste, &["Ctrl+V"]),
            (Cmd::CopyAll, &["Ctrl+Shift+C*"]),
            (Cmd::DeleteLine, &["Ctrl+Shift+L"]),
            (Cmd::Undo, &["Ctrl+Z"]),
            (Cmd::Redo, &["Ctrl+Y"]),
            (Cmd::SelectAll, &["Ctrl+A"]),
            (Cmd::LineStart, &["Home"]),
            (Cmd::LineEnd, &["End"]),
            (Cmd::Help, &["F1*"]),
        ],
    },
    Profile {
        key: "emacs",
        name: "Emacs",
        binds: &[
            (Cmd::Save, &["Ctrl+X Ctrl+S"]),
            (Cmd::SaveAs, &["Ctrl+X Ctrl+W"]),
            (Cmd::Open, &["Ctrl+X Ctrl+F"]),
            (Cmd::InsertFile, &["Ctrl+X I"]),
            (Cmd::CloseTab, &["Ctrl+X K"]),
            (Cmd::Find, &["Ctrl+S"]),
            (Cmd::Replace, &["Alt+Shift+Digit5"]),
            (Cmd::FindNext, &["F3*"]),
            (Cmd::FindPrev, &["Ctrl+R"]),
            (Cmd::GotoLine, &["Alt+G G", "Alt+G Alt+G"]),
            (Cmd::MatchingBracket, &["Ctrl+Alt+F"]),
            (Cmd::CutLine, &["Ctrl+K", "Ctrl+W"]),
            (Cmd::CopyLine, &["Alt+W"]),
            (Cmd::Paste, &["Ctrl+Y"]),
            (Cmd::CopyAll, &["Ctrl+Shift+C*"]),
            (Cmd::DeleteLine, &["Ctrl+Shift+Backspace"]),
            (Cmd::Undo, &["Ctrl+Slash", "Ctrl+X U"]),
            (Cmd::Redo, &["Ctrl+Shift+Slash"]),
            (Cmd::SelectAll, &["Ctrl+X H"]),
            (Cmd::Mark, &["Ctrl+Space"]),
            (Cmd::LineStart, &["Ctrl+A"]),
            (Cmd::LineEnd, &["Ctrl+E"]),
            (Cmd::Help, &["Ctrl+H B"]),
        ],
    },
    Profile {
        key: "nano",
        name: "nano",
        binds: &[
            (Cmd::Save, &["Ctrl+S"]),
            (Cmd::SaveAs, &["Ctrl+O"]),
            (Cmd::Open, &["Ctrl+Shift+O*"]),
            (Cmd::InsertFile, &["Ctrl+R"]),
            (Cmd::CloseTab, &["Ctrl+X"]),
            (Cmd::Find, &["Ctrl+W", "Ctrl+F"]),
            (Cmd::Replace, &["Alt+R", "Ctrl+Backslash"]),
            (Cmd::FindNext, &["Alt+W", "F3*"]),
            (Cmd::FindPrev, &["Ctrl+Q", "Ctrl+B"]),
            (Cmd::GotoLine, &["Alt+G", "Ctrl+Shift+Minus"]),
            (Cmd::MatchingBracket, &["Alt+CloseBracket"]),
            (Cmd::CutLine, &["Ctrl+K"]),
            (Cmd::CopyLine, &["Alt+Digit6"]),
            (Cmd::Paste, &["Ctrl+U"]),
            (Cmd::CopyAll, &["Ctrl+Shift+C*"]),
            (Cmd::DeleteLine, &["Alt+Delete"]),
            (Cmd::Undo, &["Alt+U"]),
            (Cmd::Redo, &["Alt+E"]),
            (Cmd::SelectAll, &["Ctrl+Shift+A*"]),
            (Cmd::Mark, &["Alt+A", "Ctrl+Digit6"]),
            (Cmd::LineStart, &["Ctrl+A"]),
            (Cmd::LineEnd, &["Ctrl+E"]),
            (Cmd::Help, &["Ctrl+G"]),
        ],
    },
];

static CHOSEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The profile chosen in the settings (`Drafts::editor_keymap`), set once per
/// frame by the app like the editor colour scheme.
pub fn set(key: &str) {
    let i = PROFILES.iter().position(|p| p.key == key).unwrap_or(0);
    CHOSEN.store(i, std::sync::atomic::Ordering::Relaxed);
}

pub fn current() -> &'static Profile {
    &PROFILES[CHOSEN
        .load(std::sync::atomic::Ordering::Relaxed)
        .min(PROFILES.len() - 1)]
}

/// The current profile's matcher, parsed once per profile.
pub fn matcher() -> &'static Matcher {
    static CACHE: std::sync::OnceLock<Vec<Matcher>> = std::sync::OnceLock::new();
    let all = CACHE.get_or_init(|| PROFILES.iter().map(Matcher::new).collect());
    &all[CHOSEN
        .load(std::sync::atomic::Ordering::Relaxed)
        .min(all.len() - 1)]
}

/// The keys of `cmd` in the current profile, for hover texts ("Ctrl+S").
pub fn keys_of(cmd: Cmd) -> String {
    let seqs: Vec<String> = bindings(current())
        .into_iter()
        .filter(|(c, _)| *c == cmd)
        .map(|(_, s)| s.label())
        .collect();
    seqs.join(" / ")
}

/// The profile saved under `key`; VS Code for anything else.
#[cfg(test)]
pub fn profile(key: &str) -> &'static Profile {
    PROFILES
        .iter()
        .find(|p| p.key == key)
        .unwrap_or(&PROFILES[0])
}

/// One key press: modifiers and key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stroke {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

impl Stroke {
    fn of(m: Modifiers, key: Key) -> Stroke {
        Stroke {
            ctrl: m.ctrl || m.command || m.mac_cmd,
            alt: m.alt,
            shift: m.shift,
            key,
        }
    }

    /// "Ctrl+Shift+K".
    pub fn parse(s: &str) -> Option<Stroke> {
        let mut st = Stroke {
            ctrl: false,
            alt: false,
            shift: false,
            key: Key::A,
        };
        let parts: Vec<&str> = s.split('+').collect();
        let (last, mods) = parts.split_last()?;
        for m in mods {
            match *m {
                "Ctrl" => st.ctrl = true,
                "Alt" => st.alt = true,
                "Shift" => st.shift = true,
                _ => return None,
            }
        }
        st.key = Key::from_name(last)?;
        Some(st)
    }

    /// How the help list shows it ("Ctrl+\\", "Alt+6", "Ctrl+_").
    pub fn label(self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        let key = match (self.key, self.shift) {
            (Key::Minus, true) => return format!("{s}_"),
            (Key::Num5, true) if self.alt => return format!("{s}%"),
            (Key::Slash, true) => return format!("{s}?"),
            (k, _) => k.symbol_or_name().to_string(),
        };
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(&key);
        s
    }
}

/// A key sequence ("Ctrl+X Ctrl+S" = two strokes) and whether Telepad chose it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seq {
    pub strokes: Vec<Stroke>,
    pub own: bool,
}

pub fn parse_seq(s: &str) -> Option<Seq> {
    let (body, own) = match s.strip_suffix('*') {
        Some(b) => (b, true),
        None => (s, false),
    };
    let strokes: Option<Vec<Stroke>> = body.split(' ').map(Stroke::parse).collect();
    let strokes = strokes?;
    (!strokes.is_empty()).then_some(Seq { strokes, own })
}

impl Seq {
    pub fn label(&self) -> String {
        let keys: Vec<String> = self.strokes.iter().map(|k| k.label()).collect();
        keys.join(" ")
    }
}

/// Every (command, sequence) of a profile, parsed.
pub fn bindings(p: &Profile) -> Vec<(Cmd, Seq)> {
    p.binds
        .iter()
        .flat_map(|(c, seqs)| seqs.iter().filter_map(move |s| Some((*c, parse_seq(s)?))))
        .collect()
}

fn native(cmd: Cmd, seq: &Seq) -> bool {
    NATIVE
        .iter()
        .any(|(c, s)| *c == cmd && parse_seq(s).as_ref() == Some(seq))
}

/// Takes a profile's shortcuts out of the frame's input.
pub struct Matcher {
    binds: Vec<(Cmd, Seq)>,
}

impl Matcher {
    pub fn new(p: &Profile) -> Matcher {
        let binds = bindings(p)
            .into_iter()
            .filter(|(c, s)| !native(*c, s))
            .collect();
        Matcher { binds }
    }

    /// Feed the frame's input: returns the commands typed, in order.
    /// `pending` carries the first stroke(s) of a sequence between frames;
    /// `text` = the text field has the focus (text commands only then).
    /// Matched keys, and the text a typed Alt+key would insert, are removed;
    /// Ctrl+X / C / V arrive as clipboard events and count as those keys.
    pub fn take(
        &self,
        input: &mut egui::InputState,
        pending: &mut Vec<Stroke>,
        text: bool,
    ) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        let mut keep = Vec::with_capacity(input.events.len());
        let mut eat_text = false;
        for ev in input.events.drain(..) {
            let stroke = match &ev {
                Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some(Stroke::of(*modifiers, *key)),
                Event::Cut => Some(Stroke::of(Modifiers::CTRL, Key::X)),
                Event::Copy => Some(Stroke::of(Modifiers::CTRL, Key::C)),
                Event::Paste(_) => Some(Stroke::of(Modifiers::CTRL, Key::V)),
                Event::Text(_) if eat_text => {
                    eat_text = false;
                    continue;
                }
                _ => None,
            };
            eat_text = false;
            let Some(st) = stroke else {
                keep.push(ev);
                continue;
            };
            // Modifier-only and plain keys only matter inside a sequence.
            let mut seq = pending.clone();
            seq.push(st);
            let usable = |c: Cmd| text || !c.on_text();
            let exact = self
                .binds
                .iter()
                .find(|(c, s)| s.strokes == seq && usable(*c))
                .map(|(c, _)| *c);
            let prefix = self.binds.iter().any(|(c, s)| {
                s.strokes.len() > seq.len() && s.strokes.starts_with(&seq) && usable(*c)
            });
            if let Some(c) = exact {
                cmds.push(c);
                pending.clear();
                eat_text = st.alt || st.ctrl || !seq.is_empty();
            } else if prefix {
                *pending = seq;
                eat_text = true;
            } else if !pending.is_empty() {
                // A sequence that does not exist: dropped, like Emacs does.
                pending.clear();
                eat_text = true;
            } else {
                keep.push(ev);
            }
        }
        input.events = keep;
        cmds
    }
}

/// The help list: (command, keys) for every command the profile binds.
pub fn help_rows(p: &Profile) -> Vec<(Cmd, String)> {
    Cmd::ALL
        .iter()
        .filter_map(|c| {
            let seqs: Vec<String> = bindings(p)
                .into_iter()
                .filter(|(b, _)| b == c)
                .map(|(_, s)| {
                    if s.own {
                        format!("{} (Telepad)", s.label())
                    } else {
                        s.label()
                    }
                })
                .collect();
            (!seqs.is_empty()).then(|| (*c, seqs.join(", ")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_profile_binds_every_command_once_without_conflicts() {
        for p in &PROFILES {
            for (c, seqs) in p.binds {
                for s in *seqs {
                    assert!(parse_seq(s).is_some(), "{}: cannot read {s:?}", p.name);
                }
                let _ = c;
            }
            let b = bindings(p);
            for c in Cmd::ALL {
                if c == Cmd::Mark && !matches!(p.key, "emacs" | "nano") {
                    continue;
                }
                assert!(
                    b.iter().any(|(x, _)| *x == c),
                    "{}: {c:?} has no key",
                    p.name
                );
            }
            // No sequence twice, and none is the start of another.
            for (i, (c1, s1)) in b.iter().enumerate() {
                for (c2, s2) in b.iter().skip(i + 1) {
                    assert!(
                        !s2.strokes.starts_with(&s1.strokes)
                            && !s1.strokes.starts_with(&s2.strokes),
                        "{}: {c1:?} {} vs {c2:?} {}",
                        p.name,
                        s1.label(),
                        s2.label()
                    );
                }
            }
        }
        assert_eq!(profile("nope").key, "vscode");
    }

    fn key(m: Modifiers, k: Key) -> Event {
        Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: m,
        }
    }

    fn run(
        p: &str,
        events: Vec<Event>,
        pending: &mut Vec<Stroke>,
        text: bool,
    ) -> (Vec<Cmd>, usize) {
        let mut input = egui::InputState::default();
        input.events = events;
        let cmds = Matcher::new(profile(p)).take(&mut input, pending, text);
        (cmds, input.events.len())
    }

    #[test]
    fn matcher_takes_keys_sequences_and_clipboard_events() {
        let mut pending = Vec::new();
        // nano: Ctrl+K cuts the line; Alt+6 copies and its "6" is not typed.
        let (c, left) = run(
            "nano",
            vec![
                key(Modifiers::CTRL, Key::K),
                key(Modifiers::ALT, Key::Num6),
                Event::Text("6".into()),
            ],
            &mut pending,
            true,
        );
        assert_eq!((c, left), (vec![Cmd::CutLine, Cmd::CopyLine], 0));
        // nano Ctrl+X arrives as a Cut event: close tab, not cut.
        assert_eq!(
            run("nano", vec![Event::Cut], &mut pending, true).0,
            [Cmd::CloseTab]
        );
        // Emacs: Ctrl+X (a Cut event) waits for the next key, across frames.
        let (c, _) = run("emacs", vec![Event::Cut], &mut pending, true);
        assert!(c.is_empty() && pending.len() == 1);
        let (c, _) = run(
            "emacs",
            vec![key(Modifiers::CTRL, Key::S)],
            &mut pending,
            true,
        );
        assert_eq!((c, pending.len()), (vec![Cmd::Save], 0));
        // An unknown second key cancels the sequence and is not typed.
        run("emacs", vec![Event::Cut], &mut pending, true);
        let (c, left) = run(
            "emacs",
            vec![key(Modifiers::NONE, Key::Q), Event::Text("q".into())],
            &mut pending,
            true,
        );
        assert_eq!((c.len(), left, pending.len()), (0, 0, 0));
        // VS Code: Ctrl+Z / Ctrl+V / Home stay with the text field.
        let (c, left) = run(
            "vscode",
            vec![
                key(Modifiers::CTRL, Key::Z),
                Event::Paste("x".into()),
                key(Modifiers::NONE, Key::Home),
            ],
            &mut pending,
            true,
        );
        assert_eq!((c.len(), left), (0, 3));
        assert_eq!(
            run(
                "vscode",
                vec![key(Modifiers::CTRL | Modifiers::SHIFT, Key::Backslash)],
                &mut pending,
                true
            )
            .0,
            [Cmd::MatchingBracket]
        );
        // Text commands wait for the text field: Ctrl+C in the find box copies normally.
        let (c, left) = run("vscode", vec![Event::Copy], &mut pending, false);
        assert_eq!((c.len(), left), (0, 1));
        assert_eq!(
            run(
                "vscode",
                vec![key(Modifiers::CTRL, Key::S)],
                &mut pending,
                false
            )
            .0,
            [Cmd::Save]
        );
    }

    /// The user manual's table: `cargo test -p telepad-gui manual_table -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn manual_table() {
        let mut s = String::from("| Command |");
        for p in &PROFILES {
            s.push_str(&format!(" {} |", p.name));
        }
        s.push_str("\n|---|");
        s.push_str(&"---|".repeat(PROFILES.len()));
        for c in Cmd::ALL {
            s.push_str(&format!("\n| {} |", c.name()));
            for p in &PROFILES {
                let k = help_rows(p)
                    .into_iter()
                    .find(|(x, _)| *x == c)
                    .map_or("—".to_string(), |(_, k)| k.replace(" (Telepad)", "*"));
                s.push_str(&format!(" {} |", k.replace('|', "\\|")));
            }
        }
        println!("{s}");
    }

    #[test]
    fn help_lists_keys_and_marks_telepad_choices() {
        let rows = help_rows(profile("nano"));
        let get = |c: Cmd| {
            rows.iter()
                .find(|(x, _)| *x == c)
                .map(|(_, k)| k.clone())
                .unwrap()
        };
        assert_eq!(get(Cmd::GotoLine), "Alt+G, Ctrl+_");
        assert_eq!(get(Cmd::CopyLine), "Alt+6");
        assert_eq!(get(Cmd::CopyAll), "Ctrl+Shift+C (Telepad)");
        assert_eq!(get(Cmd::Replace), "Alt+R, Ctrl+\\");
        let vs = help_rows(profile("vscode"));
        assert!(
            vs.iter()
                .any(|(c, k)| *c == Cmd::MatchingBracket && k == "Ctrl+Shift+\\")
        );
        assert_eq!(
            help_rows(profile("emacs"))
                .iter()
                .find(|(c, _)| *c == Cmd::Save)
                .unwrap()
                .1,
            "Ctrl+X Ctrl+S"
        );
    }
}
