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
            path: ctx.state.tab.path.clone(),
            select: ctx.state.tab.selected().map(|entry| entry.name.clone()),
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
