use hwe_vfs::RemoveMode;

use crate::command::{Args, CommandContext, CommandHandler, CommandSpec};
use crate::effect::Effect;
use crate::state::{PromptKind, PromptState, StatusLevel};

pub struct RefreshCommand;

impl CommandHandler for RefreshCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.refresh",
            description: "Reload the current directory",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::Navigate {
            path: ctx.state.tab().path.clone(),
            select: ctx.state.selected().map(|entry| entry.name.clone()),
        }
    }
}

pub struct QuitCommand;

impl CommandHandler for QuitCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.quit",
            description: "Quit the file manager",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::Quit
    }
}

pub struct ThemeCommand;

impl CommandHandler for ThemeCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "view.theme",
            description: "Cycle the color theme",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::CycleTheme
    }
}

pub struct IconsCommand;

impl CommandHandler for IconsCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "view.icons",
            description: "Toggle file-type icons",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::ToggleIcons
    }
}

pub struct PreviewFocusCommand;

impl CommandHandler for PreviewFocusCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "preview.focus",
            description: "Focus or unfocus the preview pane",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        ctx.state.preview.focus = !ctx.state.preview.focus;
        Effect::Redraw
    }
}

pub struct HelpCommand;

impl CommandHandler for HelpCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.help",
            description: "Show or hide the keymap",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        ctx.state.help = !ctx.state.help;
        Effect::Redraw
    }
}

pub struct NewTabCommand;

impl CommandHandler for NewTabCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "tab.new",
            description: "Open a new tab at the current directory",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::NewTab
    }
}

pub struct CloseTabCommand;

impl CommandHandler for CloseTabCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "tab.close",
            description: "Close the current tab",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::CloseTab
    }
}

pub struct NextTabCommand;

impl CommandHandler for NextTabCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "tab.next",
            description: "Switch to the next tab",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::NextTab
    }
}

pub struct PreviousTabCommand;

impl CommandHandler for PreviousTabCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "tab.previous",
            description: "Switch to the previous tab",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::PreviousTab
    }
}

pub struct MkdirCommand;

impl CommandHandler for MkdirCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.mkdir",
            description: "Create a directory",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::Prompt(PromptState::text(PromptKind::Mkdir, "mkdir", ""))
    }
}

pub struct TouchCommand;

impl CommandHandler for TouchCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.touch",
            description: "Create an empty file",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::Prompt(PromptState::text(PromptKind::Touch, "touch", ""))
    }
}

pub struct RenameCommand;

impl CommandHandler for RenameCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.rename",
            description: "Rename the selected entry",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        let Some(entry) = ctx.state.selected() else {
            ctx.state.set_status(StatusLevel::Info, "no selection");
            return Effect::Redraw;
        };
        Effect::Prompt(PromptState::text(
            PromptKind::Rename {
                target: entry.path.clone(),
            },
            "rename",
            entry.name.clone(),
        ))
    }
}

pub struct DeleteCommand;

impl CommandHandler for DeleteCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.delete",
            description: "Delete selected entries to trash",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::DeleteSelection {
            mode: RemoveMode::Trash,
        }
    }
}

pub struct DeletePermanentCommand;

impl CommandHandler for DeletePermanentCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.delete-permanent",
            description: "Permanently delete selected entries",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::DeleteSelection {
            mode: RemoveMode::Permanent,
        }
    }
}

pub struct CopyCommand;

impl CommandHandler for CopyCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.copy",
            description: "Copy selected entries",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::CopySelection { cut: false }
    }
}

pub struct CutCommand;

impl CommandHandler for CutCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.cut",
            description: "Cut selected entries",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::CopySelection { cut: true }
    }
}

pub struct PasteCommand;

impl CommandHandler for PasteCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.paste",
            description: "Paste clipboard entries",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::PasteClipboard
    }
}

pub struct SelectAllCommand;

impl CommandHandler for SelectAllCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.select-all",
            description: "Select all entries",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::SelectAll
    }
}

pub struct ToggleMarkCommand;

impl CommandHandler for ToggleMarkCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.toggle-mark",
            description: "Toggle mark on the selected entry",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::ToggleMark
    }
}

pub struct ClearMarksCommand;

impl CommandHandler for ClearMarksCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.clear-marks",
            description: "Clear selected marks",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::ClearMarks
    }
}

pub struct ToggleHiddenCommand;

impl CommandHandler for ToggleHiddenCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "view.hidden",
            description: "Toggle hidden files",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::ToggleHidden
    }
}

pub struct CycleSortCommand;

impl CommandHandler for CycleSortCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "view.sort",
            description: "Cycle sort mode",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::CycleSort
    }
}

pub struct ToggleSortReverseCommand;

impl CommandHandler for ToggleSortReverseCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "view.sort-reverse",
            description: "Toggle reverse sort",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::ToggleSortReverse
    }
}

pub struct ToggleDirsFirstCommand;

impl CommandHandler for ToggleDirsFirstCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "view.dirs-first",
            description: "Toggle directories-first sorting",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::ToggleDirsFirst
    }
}

pub struct FilterCommand;

impl CommandHandler for FilterCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.filter",
            description: "Filter the current directory",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, ctx: &mut CommandContext<'_>, args: &Args) -> Effect {
        match args.get(0) {
            Some(query) => Effect::SetFilter(if query.is_empty() {
                None
            } else {
                Some(query.to_string())
            }),
            None => Effect::Prompt(PromptState::text(
                PromptKind::Filter,
                "filter",
                ctx.state.tab().filter.clone().unwrap_or_default(),
            )),
        }
    }
}

pub struct ClearFilterCommand;

impl CommandHandler for ClearFilterCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.clear-filter",
            description: "Clear the current filter",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::SetFilter(None)
    }
}

pub struct SearchCommand;

impl CommandHandler for SearchCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.search",
            description: "Search recursively for matching entries",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, args: &Args) -> Effect {
        match args.get(0) {
            Some(query) => Effect::Search(Some(query.to_string())),
            None => Effect::Prompt(PromptState::text(
                PromptKind::Search,
                "search",
                String::new(),
            )),
        }
    }
}

pub struct ClearSearchCommand;

impl CommandHandler for ClearSearchCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.clear-search",
            description: "Clear search results and restore the directory",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::ClearSearch
    }
}

pub struct OpenExternalCommand;

impl CommandHandler for OpenExternalCommand {
    fn spec(&self) -> &'static CommandSpec {
        static SPEC: CommandSpec = CommandSpec {
            name: "app.open",
            description: "Open the selected entry externally",
            args: &[],
        };
        &SPEC
    }

    fn run(&self, _ctx: &mut CommandContext<'_>, _args: &Args) -> Effect {
        Effect::OpenExternal
    }
}

#[cfg(test)]
mod tests {
    use hwe_vfs::VPath;

    use super::*;
    use crate::state::AppState;

    #[test]
    fn search_command_accepts_an_argument_and_prompts() {
        let mut state = AppState::new(VPath::new("/tmp"));
        let mut ctx = CommandContext { state: &mut state };
        match SearchCommand.run(&mut ctx, &Args::new(vec!["note".into()])) {
            Effect::Search(Some(query)) => assert_eq!(query, "note"),
            other => panic!("unexpected effect: {other:?}"),
        }
        assert!(matches!(
            SearchCommand.run(&mut ctx, &Args::empty()),
            Effect::Prompt(_)
        ));
        assert!(matches!(
            ClearSearchCommand.run(&mut ctx, &Args::empty()),
            Effect::ClearSearch
        ));
    }

    #[test]
    fn filter_command_accepts_an_argument() {
        let mut state = AppState::new(VPath::new("/tmp"));
        let mut ctx = CommandContext { state: &mut state };
        let effect = FilterCommand.run(&mut ctx, &Args::new(vec!["note".into()]));
        match effect {
            Effect::SetFilter(Some(query)) => assert_eq!(query, "note"),
            other => panic!("unexpected effect: {other:?}"),
        }

        let effect = FilterCommand.run(&mut ctx, &Args::empty());
        assert!(matches!(effect, Effect::Prompt(_)));
    }
}
