use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Alt(char),
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

impl Key {
    pub fn ctrl(key: char) -> Self {
        Self::Ctrl(key.to_ascii_lowercase())
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Char(' ') => f.write_str("space"),
            Self::Char(key) => write!(f, "{key}"),
            Self::Ctrl(key) => write!(f, "C-{key}"),
            Self::Alt(key) => write!(f, "M-{key}"),
            Self::Enter => f.write_str("enter"),
            Self::Esc => f.write_str("esc"),
            Self::Tab => f.write_str("tab"),
            Self::BackTab => f.write_str("backtab"),
            Self::Backspace => f.write_str("backspace"),
            Self::Delete => f.write_str("delete"),
            Self::Insert => f.write_str("insert"),
            Self::Up => f.write_str("up"),
            Self::Down => f.write_str("down"),
            Self::Left => f.write_str("left"),
            Self::Right => f.write_str("right"),
            Self::Home => f.write_str("home"),
            Self::End => f.write_str("end"),
            Self::PageUp => f.write_str("pageup"),
            Self::PageDown => f.write_str("pagedown"),
            Self::F(number) => write!(f, "F{number}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Pending,
    Command(&'static str),
    Unbound,
}

#[derive(Debug, Default)]
pub struct Keymap {
    bindings: Vec<(Vec<Key>, &'static str)>,
    pending: Vec<Key>,
}

impl Keymap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn vim() -> Self {
        let mut keymap = Self::new();
        keymap.bind(Key::Char('h'), "nav.parent");
        keymap.bind(Key::Left, "nav.parent");
        keymap.bind(Key::Char('j'), "nav.down");
        keymap.bind(Key::Down, "nav.down");
        keymap.bind(Key::Char('k'), "nav.up");
        keymap.bind(Key::Up, "nav.up");
        keymap.bind(Key::Char('l'), "nav.enter");
        keymap.bind(Key::Right, "nav.enter");
        keymap.bind(Key::Enter, "nav.enter");
        keymap.bind_sequence(&[Key::Char('g'), Key::Char('g')], "nav.top");
        keymap.bind(Key::Home, "nav.top");
        keymap.bind(Key::Char('G'), "nav.bottom");
        keymap.bind(Key::End, "nav.bottom");
        keymap.bind(Key::F(5), "app.refresh");
        keymap.bind(Key::Char('T'), "view.theme");
        keymap.bind(Key::Char('I'), "view.icons");
        keymap.bind(Key::Tab, "preview.focus");
        keymap.bind(Key::Char('?'), "app.help");
        keymap.bind(Key::ctrl('t'), "tab.new");
        keymap.bind(Key::ctrl('w'), "tab.close");
        keymap.bind_sequence(&[Key::Char('g'), Key::Char('t')], "tab.next");
        keymap.bind_sequence(&[Key::Char('g'), Key::Char('T')], "tab.previous");
        keymap.bind(Key::Char('q'), "app.quit");
        keymap.bind(Key::ctrl('c'), "app.quit");
        keymap.bind(Key::Char(' '), "app.toggle-mark");
        keymap.bind(Key::Char('m'), "app.toggle-mark");
        keymap.bind(Key::Char('d'), "app.delete");
        keymap.bind(Key::Char('D'), "app.delete-permanent");
        keymap.bind(Key::Char('y'), "app.copy");
        keymap.bind(Key::Char('x'), "app.cut");
        keymap.bind(Key::Char('p'), "app.paste");
        keymap.bind(Key::Char('r'), "app.rename");
        keymap.bind(Key::Char('n'), "app.mkdir");
        keymap.bind(Key::Char('N'), "app.touch");
        keymap.bind(Key::Char('o'), "app.open");
        keymap.bind(Key::Char('H'), "view.hidden");
        keymap.bind(Key::Char('s'), "view.sort");
        keymap.bind(Key::Char('S'), "view.sort-reverse");
        keymap.bind(Key::Char('/'), "app.filter");
        keymap.bind(Key::Char('\\'), "app.clear-filter");
        keymap.bind(Key::Char('A'), "app.select-all");
        keymap.bind(Key::Char('u'), "app.clear-marks");
        keymap
    }

    pub fn bind(&mut self, key: Key, command: &'static str) {
        self.bindings.push((vec![key], command));
    }

    pub fn bind_sequence(&mut self, keys: &[Key], command: &'static str) {
        self.bindings.push((keys.to_vec(), command));
    }

    pub fn resolve(&mut self, key: Key) -> Resolution {
        self.pending.push(key);

        loop {
            if let Some(command) = self.exact_command() {
                self.pending.clear();
                return Resolution::Command(command);
            }

            if self.is_prefix() {
                return Resolution::Pending;
            }

            if self.pending.len() == 1 {
                self.pending.clear();
                return Resolution::Unbound;
            }

            let last = self.pending.pop().expect("pending key");
            self.pending.clear();
            self.pending.push(last);
        }
    }

    pub fn pending(&self) -> &[Key] {
        &self.pending
    }

    pub fn bindings(&self) -> impl Iterator<Item = (&[Key], &'static str)> {
        self.bindings
            .iter()
            .map(|(sequence, command)| (sequence.as_slice(), *command))
    }

    fn exact_command(&self) -> Option<&'static str> {
        self.bindings
            .iter()
            .find(|(sequence, _)| sequence == &self.pending)
            .map(|(_, command)| *command)
    }

    fn is_prefix(&self) -> bool {
        self.bindings
            .iter()
            .any(|(sequence, _)| sequence.starts_with(&self.pending))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_keys_resolve() {
        let mut keymap = Keymap::vim();
        assert_eq!(
            keymap.resolve(Key::Char('j')),
            Resolution::Command("nav.down")
        );
        assert_eq!(keymap.resolve(Key::Down), Resolution::Command("nav.down"));
    }

    #[test]
    fn gg_requires_two_keys() {
        let mut keymap = Keymap::vim();
        assert_eq!(keymap.resolve(Key::Char('g')), Resolution::Pending);
        assert_eq!(keymap.pending(), &[Key::Char('g')]);
        assert_eq!(
            keymap.resolve(Key::Char('g')),
            Resolution::Command("nav.top")
        );
        assert!(keymap.pending().is_empty());
    }

    #[test]
    fn invalid_sequence_retries_last_key() {
        let mut keymap = Keymap::vim();
        assert_eq!(keymap.resolve(Key::Char('g')), Resolution::Pending);
        assert_eq!(
            keymap.resolve(Key::Char('j')),
            Resolution::Command("nav.down")
        );
    }

    #[test]
    fn unbound_key_is_reported_and_clears_pending() {
        let mut keymap = Keymap::vim();
        assert_eq!(keymap.resolve(Key::Char('z')), Resolution::Unbound);
        assert!(keymap.pending().is_empty());
    }

    #[test]
    fn tab_keys_resolve() {
        let mut keymap = Keymap::vim();
        assert_eq!(
            keymap.resolve(Key::ctrl('t')),
            Resolution::Command("tab.new")
        );
        assert_eq!(
            keymap.resolve(Key::ctrl('w')),
            Resolution::Command("tab.close")
        );
        assert_eq!(keymap.resolve(Key::Char('g')), Resolution::Pending);
        assert_eq!(
            keymap.resolve(Key::Char('t')),
            Resolution::Command("tab.next")
        );
        assert_eq!(keymap.resolve(Key::Char('g')), Resolution::Pending);
        assert_eq!(
            keymap.resolve(Key::Char('T')),
            Resolution::Command("tab.previous")
        );
    }

    #[test]
    fn uppercase_g_is_bottom() {
        let mut keymap = Keymap::vim();
        assert_eq!(
            keymap.resolve(Key::Char('G')),
            Resolution::Command("nav.bottom")
        );
    }
}
