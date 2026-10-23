//! Optional full-screen learning terminal. No background service, no editor subprocess.
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::io::{self, IsTerminal};
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Submit(String),
    Ask(String),
    Hint,
    Learn,
    Skip,
    Quit,
}

#[derive(Default)]
pub struct Editor {
    pub answer: String,
    pub ask: String,
    asking: bool,
    code_scroll: u16,
}

fn handle_key(state: &mut Editor, key: KeyEvent, practice: bool, review: bool) -> Option<Action> {
    if key.kind != KeyEventKind::Press && key.kind != KeyEventKind::Repeat {
        return None;
    }
    match (key.code, key.modifiers) {
        (KeyCode::Esc, _) => return Some(Action::Quit),
        (KeyCode::F(1), _) if !review => return Some(Action::Hint),
        (KeyCode::F(2), _) if !review => {
            state.asking = !state.asking;
            return None;
        }
        (KeyCode::F(3), _) if !review => return Some(Action::Learn),
        (KeyCode::F(4), _) => return Some(Action::Skip),
        (KeyCode::F(5), _) => {
            if !state.answer.trim().is_empty() {
                return Some(Action::Submit(state.answer.trim().to_owned()));
            }
        }
        (KeyCode::PageUp, _) => {
            state.code_scroll = state.code_scroll.saturating_sub(6);
        }
        (KeyCode::PageDown, _) => {
            state.code_scroll = state.code_scroll.saturating_add(6);
        }
        (KeyCode::Enter, _) if state.asking => {
            if !state.ask.trim().is_empty() {
                let question = std::mem::take(&mut state.ask);
                state.asking = false;
                return Some(Action::Ask(question));
            }
        }
        (KeyCode::Enter, _) if !practice && !key.modifiers.contains(KeyModifiers::SHIFT) => {
            if !state.answer.trim().is_empty() {
                return Some(Action::Submit(state.answer.trim().to_owned()));
            }
        }
        (KeyCode::Enter, _) => {
            if state.asking {
                state.ask.push(' ');
            } else {
                state.answer.push('\n');
            }
        }
        (KeyCode::Backspace, _) => {
            if state.asking {
                state.ask.pop();
            } else {
                state.answer.pop();
            }
        }
        (KeyCode::Char('c'), mods) if mods.contains(KeyModifiers::CONTROL) => {
            return Some(Action::Quit);
        }
        (KeyCode::Char(c), mods)
            if !mods.contains(KeyModifiers::CONTROL) && !mods.contains(KeyModifiers::ALT) =>
        {
            let target = if state.asking {
                &mut state.ask
            } else {
                &mut state.answer
            };
            if target.len() < 14_000 {
                target.push(c);
            }
        }
        _ => {}
    }
    None
}

struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

pub fn supported() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Display a question with always-visible source, problem and answer field.
/// F1 hint, F2 teacher, F3 explanation, F4 skip, F5 submit, Esc quit.
/// Practice uses Enter for multiline and F5 to submit.
pub struct Prompt<'a> {
    pub question: &'a str,
    pub source: &'a str,
    pub topic: &'a str,
    pub location: &'a str,
    pub practice: bool,
    pub review: bool,
    pub message: &'a str,
}

pub fn prompt(view: Prompt<'_>, state: &mut Editor) -> Result<Action, String> {
    let Prompt {
        question,
        source,
        topic,
        location,
        practice,
        review,
        message,
    } = view;
    if !supported() {
        return Err("Повноекранний режим потребує інтерактивного термінала Windows Terminal або TTY. Без нього запускай без --tui.".into());
    }
    enable_raw_mode().map_err(|e| e.to_string())?;
    let _screen = Screen;
    execute!(io::stdout(), EnterAlternateScreen).map_err(|e| e.to_string())?;
    let mut term = Terminal::new(CrosstermBackend::new(io::stdout())).map_err(|e| e.to_string())?;
    loop {
        term.draw(|frame| {
            let vertical = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Percentage(49),
                    Constraint::Percentage(20),
                    Constraint::Min(5),
                    Constraint::Length(6),
                ]).split(frame.area());
            let header = Paragraph::new(Line::from(vec![
                Span::styled(" ◆ CodeGym ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("  •  "),
                Span::styled(topic, Style::default().fg(Color::Yellow)),
                Span::raw("  •  "),
                Span::styled(location, Style::default().fg(Color::Gray)),
            ])).block(Block::default().borders(Borders::ALL).title(" Тренажер коду "));
            frame.render_widget(header, vertical[0]);
            let code = Paragraph::new(source)
                .block(Block::default().borders(Borders::ALL).title(" Реальний код — PgUp/PgDn "))
                .style(Style::default().fg(Color::LightCyan))
                .wrap(Wrap { trim: false })
                .scroll((state.code_scroll, 0));
            frame.render_widget(code, vertical[1]);
            let prompt = Paragraph::new(question)
                .block(Block::default().borders(Borders::ALL).title(" Питання "))
                .wrap(Wrap { trim: false });
            frame.render_widget(prompt, vertical[2]);
            let (body, label) = if state.asking {
                (state.ask.as_str(), " Запитання викладачу • Enter = запитати ")
            } else if practice {
                (state.answer.as_str(), " Твоя функція • Enter = новий рядок, F5 = перевірити ")
            } else {
                (state.answer.as_str(), " Твоя відповідь • Enter або F5 = перевірити ")
            };
            let entered = if body.is_empty() { Text::raw("Пиши тут…") } else { Text::raw(body) };
            let visible_lines = vertical[3].height.saturating_sub(2) as usize;
            let input_scroll = body.lines().count().saturating_sub(visible_lines) as u16;
            frame.render_widget(
                Paragraph::new(entered).block(Block::default().borders(Borders::ALL).title(label))
                    .wrap(Wrap { trim: false }).scroll((input_scroll, 0)),
                vertical[3]
            );
            let shortcuts = if review {
                "F4 пропустити  •  F5 відповісти  •  Esc вийти"
            } else {
                "F1 підказка  •  F2 запитати AI  •  F3 розбір  •  F4 пропуск  •  F5 перевірити  •  Esc"
            };
            let footer = Paragraph::new(vec![
                Line::from(Span::styled(shortcuts, Style::default().fg(Color::Green))),
                Line::from(message.chars().take(240).collect::<String>()),
            ]).block(Block::default().borders(Borders::ALL)).wrap(Wrap { trim: false });
            frame.render_widget(footer, vertical[4]);
        }).map_err(|e| format!("Помилка малювання термінала: {e}"))?;

        if event::poll(Duration::from_millis(200)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(key) => {
                    if let Some(action) = handle_key(state, key, practice, review) {
                        return Ok(action);
                    }
                }
                Event::Paste(text) => {
                    let target = if state.asking {
                        &mut state.ask
                    } else {
                        &mut state.answer
                    };
                    let space = 14_000usize.saturating_sub(target.len());
                    let clean: String = text
                        .chars()
                        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
                        .take(space)
                        .collect();
                    target.push_str(&clean);
                }
                _ => {}
            }
        }
    }
}

/// Keep the result visible before the next question replaces the screen.
/// Enter continues; Esc ends the current lesson after progress is saved.
pub fn show_feedback(topic: &str, explanation: &str) -> Result<bool, String> {
    if !supported() {
        return Err("Підсумок у TUI потребує інтерактивного термінала.".into());
    }
    enable_raw_mode().map_err(|e| e.to_string())?;
    let _screen = Screen;
    execute!(io::stdout(), EnterAlternateScreen).map_err(|e| e.to_string())?;
    let mut terminal =
        Terminal::new(CrosstermBackend::new(io::stdout())).map_err(|e| e.to_string())?;
    let mut scroll = 0u16;
    loop {
        terminal
            .draw(|frame| {
                let sections = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(5),
                        Constraint::Length(3),
                    ])
                    .split(frame.area());
                frame.render_widget(
                    Paragraph::new(format!("◆ CodeGym • {topic}"))
                        .style(
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        )
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Результат заняття "),
                        ),
                    sections[0],
                );
                frame.render_widget(
                    Paragraph::new(explanation)
                        .wrap(Wrap { trim: false })
                        .scroll((scroll, 0))
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title(" Оцінка, пояснення та правильна відповідь "),
                        ),
                    sections[1],
                );
                frame.render_widget(
                    Paragraph::new(
                        "Enter — наступне питання   PgUp/PgDn — прокрутка   Esc — завершити урок",
                    )
                    .block(Block::default().borders(Borders::ALL)),
                    sections[2],
                );
            })
            .map_err(|e| e.to_string())?;
        if event::poll(Duration::from_millis(200)).map_err(|e| e.to_string())?
            && let Event::Key(key) = event::read().map_err(|e| e.to_string())?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Enter | KeyCode::F(5) => return Ok(true),
                KeyCode::Esc => return Ok(false),
                KeyCode::PageDown | KeyCode::Down => scroll = scroll.saturating_add(5),
                KeyCode::PageUp | KeyCode::Up => scroll = scroll.saturating_sub(5),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn practice_uses_f5_to_submit_multiline_code() {
        let mut state = Editor::default();
        for c in "fn x(){}".chars() {
            handle_key(&mut state, key(KeyCode::Char(c)), true, false);
        }
        assert_eq!(
            handle_key(&mut state, key(KeyCode::Enter), true, false),
            None
        );
        assert!(state.answer.ends_with('\n'));
        assert_eq!(
            handle_key(&mut state, key(KeyCode::F(5)), true, false),
            Some(Action::Submit("fn x(){}".into()))
        );
    }

    #[test]
    fn review_disables_hint_and_tutor() {
        let mut state = Editor::default();
        assert_eq!(
            handle_key(&mut state, key(KeyCode::F(1)), false, true),
            None
        );
        assert_eq!(
            handle_key(&mut state, key(KeyCode::F(2)), false, true),
            None
        );
        assert!(!state.asking);
        assert_eq!(
            handle_key(&mut state, key(KeyCode::Esc), false, true),
            Some(Action::Quit)
        );
    }

    #[test]
    fn tutor_request_preserves_answer() {
        let mut state = Editor {
            answer: "already typed".into(),
            ..Editor::default()
        };
        handle_key(&mut state, key(KeyCode::F(2)), false, false);
        state.ask.push_str("why await?");
        assert_eq!(
            handle_key(&mut state, key(KeyCode::Enter), false, false),
            Some(Action::Ask("why await?".into()))
        );
        assert_eq!(state.answer, "already typed");
    }
}
