use hwe_vfs::EntryKind;

use crate::command::{Args, CommandContext, CommandHandler, CommandSpec};
use crate::effect::Effect;
use crate::state::StatusLevel;

pub struct UpCommand;

impl CommandHandler for UpCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "nav.up",
            description: "Move the cursor up one entry",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        let settings = ctx.state.settings;
        if ctx.state.tab_mut().move_cursor(-1, &settings) {
            Effect::Redraw
        } else {
            Effect::None
        }
    }
}

pub struct DownCommand;

impl CommandHandler for DownCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "nav.down",
            description: "Move the cursor down one entry",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        let settings = ctx.state.settings;
        if ctx.state.tab_mut().move_cursor(1, &settings) {
            Effect::Redraw
        } else {
            Effect::None
        }
    }
}

pub struct TopCommand;

impl CommandHandler for TopCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "nav.top",
            description: "Jump to the first entry",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        let settings = ctx.state.settings;
        if ctx.state.tab_mut().jump_top(&settings) {
            Effect::Redraw
        } else {
            Effect::None
        }
    }
}

pub struct BottomCommand;

impl CommandHandler for BottomCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "nav.bottom",
            description: "Jump to the last entry",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        let settings = ctx.state.settings;
        if ctx.state.tab_mut().jump_bottom(&settings) {
            Effect::Redraw
        } else {
            Effect::None
        }
    }
}

pub struct ParentCommand;

impl CommandHandler for ParentCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "nav.parent",
            description: "Open the parent directory",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        let current = ctx.state.tab().path.clone();
        match current.parent() {
            Some(parent) => {
                let select = current
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned());
                Effect::Navigate {
                    path: parent,
                    select,
                }
            }
            None => {
                ctx.state
                    .set_status(StatusLevel::Info, "already at filesystem root");
                Effect::Redraw
            }
        }
    }
}

pub struct EnterCommand;

impl CommandHandler for EnterCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "nav.enter",
            description: "Enter the hovered directory",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        let Some(entry) = ctx.state.selected() else {
            return Effect::None;
        };

        if entry.kind == EntryKind::Dir {
            Effect::Navigate {
                path: entry.path.clone(),
                select: None,
            }
        } else {
            let message = format!("{} is not a directory", entry.name);
            ctx.state.set_status(StatusLevel::Info, message);
            Effect::Redraw
        }
    }
}

#[cfg(test)]
mod tests {
    use hwe_vfs::VPath;

    use super::*;
    use crate::command::{Args, CommandContext, CommandHandler};
    use crate::state::{AppState, Entry, StatusLevel};

    fn entry(name: &str, kind: EntryKind) -> Entry {
        Entry {
            name: name.to_string(),
            path: VPath::new(format!("/tmp/{name}")),
            kind,
            size: None,
            modified: None,
            is_hidden: false,
            is_symlink: false,
            is_executable: false,
            mode: None,
        }
    }

    fn state_with(entries: Vec<Entry>) -> AppState {
        let mut state = AppState::new(VPath::new("/tmp"));
        state.tab_mut().entries = entries;
        state.tab_mut().loading = false;
        state
    }

    #[test]
    fn enter_directory_navigates() {
        let mut state = state_with(vec![entry("sub", EntryKind::Dir)]);
        let mut ctx = CommandContext { state: &mut state };
        let effect = EnterCommand.run(&mut ctx, &Args::empty());
        match effect {
            Effect::Navigate { path, select } => {
                assert_eq!(path, VPath::new("/tmp/sub"));
                assert_eq!(select, None);
            }
            other => panic!("unexpected effect: {other:?}"),
        }
    }

    #[test]
    fn enter_file_reports_status() {
        let mut state = state_with(vec![entry("notes.txt", EntryKind::File)]);
        let mut ctx = CommandContext { state: &mut state };
        let effect = EnterCommand.run(&mut ctx, &Args::empty());
        assert!(matches!(effect, Effect::Redraw));
        let status = ctx.state.status.as_ref().expect("status");
        assert_eq!(status.level, StatusLevel::Info);
        assert!(status.text.contains("not a directory"));
    }

    #[test]
    fn parent_navigates_and_selects_previous_directory() {
        let mut state = AppState::new(VPath::new("/tmp/current"));
        state.tab_mut().loading = false;
        let mut ctx = CommandContext { state: &mut state };
        let effect = ParentCommand.run(&mut ctx, &Args::empty());
        match effect {
            Effect::Navigate { path, select } => {
                assert_eq!(path, VPath::new("/tmp"));
                assert_eq!(select.as_deref(), Some("current"));
            }
            other => panic!("unexpected effect: {other:?}"),
        }
    }

    #[test]
    fn parent_at_root_reports_status() {
        let mut state = AppState::new(VPath::new("/"));
        state.tab_mut().loading = false;
        let mut ctx = CommandContext { state: &mut state };
        let effect = ParentCommand.run(&mut ctx, &Args::empty());
        assert!(matches!(effect, Effect::Redraw));
        assert!(ctx.state.status.is_some());
    }
}
