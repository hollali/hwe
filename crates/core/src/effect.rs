use hwe_vfs::VPath;

#[derive(Clone, Debug)]
pub enum Effect {
    None,
    Redraw,
    Navigate { path: VPath, select: Option<String> },
    CycleTheme,
    ToggleIcons,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    Quit,
}
