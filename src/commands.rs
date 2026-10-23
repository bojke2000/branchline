use crate::app::{Direction, EditorAction};
use eframe::egui::{Key, Modifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppCommand {
    New,
    Open,
    Save,
    SaveAs,
    Export,
    Quit,
    AddChild,
    AddSibling,
    Edit,
    Fold,
    MoveUp,
    MoveDown,
    Indent,
    Outdent,
    Delete,
    Undo,
    Redo,
    Left,
    Right,
    Up,
    Down,
    Search,
    Palette,
    Notes,
    Fit,
    Focus,
    Theme,
    Help,
}

pub const EDIT_COMMANDS: &[AppCommand] = &[
    AppCommand::AddChild,
    AppCommand::AddSibling,
    AppCommand::Edit,
    AppCommand::Fold,
    AppCommand::MoveUp,
    AppCommand::MoveDown,
    AppCommand::Indent,
    AppCommand::Outdent,
    AppCommand::Delete,
    AppCommand::Undo,
    AppCommand::Redo,
];
pub const NAVIGATION_COMMANDS: &[AppCommand] = &[
    AppCommand::Left,
    AppCommand::Right,
    AppCommand::Up,
    AppCommand::Down,
];
pub const FILE_COMMANDS: &[AppCommand] = &[
    AppCommand::New,
    AppCommand::Open,
    AppCommand::Save,
    AppCommand::SaveAs,
    AppCommand::Export,
    AppCommand::Quit,
];
pub const VIEW_COMMANDS: &[AppCommand] = &[
    AppCommand::Search,
    AppCommand::Palette,
    AppCommand::Notes,
    AppCommand::Fit,
    AppCommand::Focus,
    AppCommand::Theme,
    AppCommand::Help,
];

#[derive(Clone, Copy)]
enum Binding {
    Plain(Key),
    Alt(Key),
    AltOrShift(Key),
    Command(Key),
    CommandShift(Key),
}

impl Binding {
    fn matches(self, key: Key, modifiers: Modifiers) -> bool {
        match self {
            Self::Plain(bound) => {
                key == bound && !modifiers.command && !modifiers.alt && !modifiers.shift
            }
            Self::Alt(bound) => key == bound && modifiers.alt && !modifiers.command,
            Self::AltOrShift(bound) => {
                key == bound && (modifiers.alt || modifiers.shift) && !modifiers.command
            }
            Self::Command(bound) => {
                key == bound && modifiers.command && !modifiers.shift && !modifiers.alt
            }
            Self::CommandShift(bound) => {
                key == bound && modifiers.command && modifiers.shift && !modifiers.alt
            }
        }
    }
}

#[derive(Clone, Copy)]
pub struct CommandSpec {
    pub action: AppCommand,
    pub label: &'static str,
    pub shortcut: &'static str,
    binding: Option<Binding>,
}

impl AppCommand {
    pub fn spec(self) -> CommandSpec {
        use AppCommand::*;
        use Binding::{AltOrShift, Command, CommandShift, Plain};
        let (label, shortcut, binding) = match self {
            New => ("New map", "Cmd/Ctrl+N", Some(Command(Key::N))),
            Open => ("Open map...", "Cmd/Ctrl+O", Some(Command(Key::O))),
            Save => ("Save", "Cmd/Ctrl+S", Some(Command(Key::S))),
            SaveAs => ("Save as...", "Cmd/Ctrl+Shift+S", Some(CommandShift(Key::S))),
            Export => ("Export Freeplane...", "", None),
            Quit => ("Quit BMM", "Cmd/Ctrl+Q", Some(Command(Key::Q))),
            AddChild => ("Add child", "Tab", Some(Plain(Key::Tab))),
            AddSibling => ("Add sibling", "Enter", Some(Plain(Key::Enter))),
            Edit => ("Edit idea", "F2", Some(Plain(Key::F2))),
            Fold => ("Fold / unfold branch", "Space", Some(Plain(Key::Space))),
            MoveUp => (
                "Move earlier",
                "Shift/Alt+Up",
                Some(AltOrShift(Key::ArrowUp)),
            ),
            MoveDown => (
                "Move later",
                "Shift/Alt+Down",
                Some(AltOrShift(Key::ArrowDown)),
            ),
            Indent => (
                "Indent under previous sibling",
                "Alt+Right",
                Some(Binding::Alt(Key::ArrowRight)),
            ),
            Outdent => (
                "Outdent to parent level",
                "Alt+Left",
                Some(Binding::Alt(Key::ArrowLeft)),
            ),
            Delete => (
                "Delete idea",
                "Delete / Backspace",
                Some(Plain(Key::Delete)),
            ),
            Undo => ("Undo", "Cmd/Ctrl+Z", Some(Command(Key::Z))),
            Redo => ("Redo", "Cmd/Ctrl+Shift+Z", Some(CommandShift(Key::Z))),
            Left => ("Navigate left", "Left", Some(Plain(Key::ArrowLeft))),
            Right => ("Navigate right", "Right", Some(Plain(Key::ArrowRight))),
            Up => ("Navigate up", "Up", Some(Plain(Key::ArrowUp))),
            Down => ("Navigate down", "Down", Some(Plain(Key::ArrowDown))),
            Search => ("Search ideas", "Cmd/Ctrl+F", Some(Command(Key::F))),
            Palette => ("Command palette", "Cmd/Ctrl+K", Some(Command(Key::K))),
            Notes => ("Show / hide notes", "", None),
            Fit => ("Fit map", "", None),
            Focus => ("Focus selection", "", None),
            Theme => ("Switch light / dark theme", "", None),
            Help => ("Keyboard guide", "", None),
        };
        CommandSpec {
            action: self,
            label,
            shortcut,
            binding,
        }
    }

    pub fn editor_action(self) -> Option<EditorAction> {
        use AppCommand::*;
        Some(match self {
            AddChild => EditorAction::AddChild,
            AddSibling => EditorAction::AddSibling,
            Edit => EditorAction::Edit,
            Fold => EditorAction::Fold,
            MoveUp => EditorAction::MoveUp,
            MoveDown => EditorAction::MoveDown,
            Indent => EditorAction::Indent,
            Outdent => EditorAction::Outdent,
            Delete => EditorAction::Delete,
            Undo => EditorAction::Undo,
            Redo => EditorAction::Redo,
            Left => EditorAction::Navigate(Direction::Left),
            Right => EditorAction::Navigate(Direction::Right),
            Up => EditorAction::Navigate(Direction::Up),
            Down => EditorAction::Navigate(Direction::Down),
            _ => return None,
        })
    }

    pub fn toolbar_label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Open => "Open",
            Self::AddChild => "Child",
            Self::AddSibling => "Sibling",
            Self::Edit => "Edit",
            Self::Fold => "Fold",
            Self::Delete => "Delete",
            Self::Search => "Search",
            Self::Palette => "Commands",
            Self::Notes => "Notes",
            Self::Fit => "Fit",
            Self::Focus => "Focus",
            Self::Theme => "Theme",
            _ => self.spec().label,
        }
    }
}

fn all_commands() -> impl Iterator<Item = AppCommand> {
    EDIT_COMMANDS
        .iter()
        .chain(NAVIGATION_COMMANDS)
        .chain(FILE_COMMANDS)
        .chain(VIEW_COMMANDS)
        .copied()
}

pub fn palette_matches(query: &str) -> Vec<CommandSpec> {
    let query = query.to_lowercase();
    all_commands()
        .map(AppCommand::spec)
        .filter(|command| command.label.to_lowercase().contains(&query))
        .collect()
}

pub fn shortcut(key: Key, modifiers: Modifiers) -> Option<AppCommand> {
    let key = if key == Key::Backspace {
        Key::Delete
    } else {
        key
    };
    all_commands().find(|command| {
        command
            .spec()
            .binding
            .is_some_and(|binding| binding.matches(key, modifiers))
    })
}
