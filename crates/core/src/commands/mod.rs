mod app;
mod navigation;

pub use app::{QuitCommand, RefreshCommand};
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
    registry.register(QuitCommand);
    registry
}
