use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use hwe_core::event::AppEvent;
use hwe_core::keymap::Key;
use tokio::sync::mpsc::UnboundedSender;

fn convert_key(event: KeyEvent) -> Option<Key> {
    if event
        .modifiers
        .intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META)
    {
        return None;
    }
    let control = event.modifiers.contains(KeyModifiers::CONTROL);
    let alt = event.modifiers.contains(KeyModifiers::ALT);
    if control && alt {
        return None;
    }

    match event.code {
        KeyCode::Char(key) if control => Some(Key::ctrl(key)),
        KeyCode::Char(key) if alt => Some(Key::Alt(key)),
        KeyCode::Char(key) => Some(Key::Char(key)),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Delete => Some(Key::Delete),
        KeyCode::Insert => Some(Key::Insert),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::PageUp => Some(Key::PageUp),
        KeyCode::PageDown => Some(Key::PageDown),
        KeyCode::F(number) => Some(Key::F(number)),
        _ => None,
    }
}

pub fn spawn_input(sender: UnboundedSender<AppEvent>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        loop {
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => {}
                Ok(false) => {
                    if sender.is_closed() {
                        break;
                    }
                    continue;
                }
                Err(_) => break,
            }

            match event::read() {
                Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                    if let Some(key) = convert_key(key)
                        && sender.send(AppEvent::Key(key)).is_err()
                    {
                        break;
                    }
                }
                Ok(Event::Resize(width, height)) => {
                    if sender.send(AppEvent::Resize { width, height }).is_err() {
                        break;
                    }
                }
                Ok(Event::Paste(text)) => {
                    if sender.send(AppEvent::Paste(text)).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Press)
    }

    #[test]
    fn modifier_keys_do_not_trigger_plain_bindings() {
        assert_eq!(
            convert_key(key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Key::Char('q'))
        );
        assert_eq!(
            convert_key(key(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Some(Key::ctrl('q'))
        );
        assert_eq!(
            convert_key(key(KeyCode::Char('q'), KeyModifiers::ALT)),
            Some(Key::Alt('q'))
        );
        assert_eq!(
            convert_key(key(KeyCode::Char('q'), KeyModifiers::SUPER)),
            None
        );
        assert_eq!(
            convert_key(key(
                KeyCode::Char('q'),
                KeyModifiers::CONTROL | KeyModifiers::ALT
            )),
            None
        );
    }
}
