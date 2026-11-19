use crate::keymap::Key;

#[derive(Clone, Debug)]
pub enum AppEvent {
    Key(Key),
    Resize { width: u16, height: u16 },
    Paste(String),
}
