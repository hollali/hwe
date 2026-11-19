mod app;
mod navigation;

pub use app::{
    CloseTabCommand, HelpCommand, IconsCommand, NewTabCommand, NextTabCommand, PreviewFocusCommand,
    PreviousTabCommand, QuitCommand, RefreshCommand, ThemeCommand,
};
pub use navigation::{
    BottomCommand, DownCommand, EnterCommand, ParentCommand, TopCommand, UpCommand,
};

use crate::command::CommandRegistry;

pub fn registry() -> CommandRegistry {
    let mut registry = CommandRegistry::new();
    registry.register(UpCommand);
    registry.register(DownCommand);
    registry.register(ParentCommand);
    registry.register(EnterCommand);
    registry.register(TopCommand);
    registry.register(BottomCommand);
    registry.register(RefreshCommand);
    registry.register(ThemeCommand);
    registry.register(IconsCommand);
    registry.register(PreviewFocusCommand);
    registry.register(HelpCommand);
    registry.register(NewTabCommand);
    registry.register(CloseTabCommand);
    registry.register(NextTabCommand);
    registry.register(PreviousTabCommand);
    registry.register(QuitCommand);
    registry
}
