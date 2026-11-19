use crate::command::{Args, CommandContext, CommandHandler, CommandSpec};
use crate::effect::Effect;

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
            select: ctx.state.tab().selected().map(|entry| entry.name.clone()),
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
