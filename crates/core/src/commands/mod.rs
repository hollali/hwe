mod app;
mod navigation;

pub use app::{
    ClearFilterCommand, ClearMarksCommand, CloseTabCommand, CopyCommand, CutCommand,
    CycleSortCommand, DeleteCommand, DeletePermanentCommand, FilterCommand, HelpCommand,
    IconsCommand, MkdirCommand, NewTabCommand, NextTabCommand, OpenExternalCommand, PasteCommand,
    PreviewFocusCommand, PreviousTabCommand, QuitCommand, RefreshCommand, RenameCommand,
    SelectAllCommand, ThemeCommand, ToggleDirsFirstCommand, ToggleHiddenCommand, ToggleMarkCommand,
    ToggleSortReverseCommand, TouchCommand,
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
    registry.register(MkdirCommand);
    registry.register(TouchCommand);
    registry.register(RenameCommand);
    registry.register(DeleteCommand);
    registry.register(DeletePermanentCommand);
    registry.register(CopyCommand);
    registry.register(CutCommand);
    registry.register(PasteCommand);
    registry.register(SelectAllCommand);
    registry.register(ClearMarksCommand);
    registry.register(ToggleMarkCommand);
    registry.register(ToggleHiddenCommand);
    registry.register(CycleSortCommand);
    registry.register(ToggleSortReverseCommand);
    registry.register(ToggleDirsFirstCommand);
    registry.register(FilterCommand);
    registry.register(ClearFilterCommand);
    registry.register(OpenExternalCommand);
    registry
}
