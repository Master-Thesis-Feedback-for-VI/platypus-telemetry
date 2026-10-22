use std::{
    collections::VecDeque,
    fs::File,
    io,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
};
use ratatui::{
    DefaultTerminal,
    layout::{Constraint, Layout},
    style::{Color, Style},
    text::Line,
    widgets::{Block, List, ListItem, ListState, Paragraph},
};
use tui_textarea::TextArea;
use unicode_width::UnicodeWidthChar;

use crate::{LogData, environment::participant_file_name, shared_data::DeviceType, write_to_file};

const LOG_HISTORY: usize = 2_000;

pub struct TerminalSession {
    pub(crate) terminal: DefaultTerminal,
}

impl TerminalSession {
    pub(crate) fn new() -> io::Result<Self> {
        let mut session = Self {
            terminal: ratatui::try_init()?,
        };
        execute!(
            session.terminal.backend_mut(),
            EnableBracketedPaste,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        Ok(session)
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = execute!(
            self.terminal.backend_mut(),
            PopKeyboardEnhancementFlags,
            DisableBracketedPaste
        );
        ratatui::restore();
    }
}

pub fn ask_participant(terminal: &mut DefaultTerminal) -> io::Result<Option<String>> {
    let mut input = TextArea::default();
    input.set_block(Block::bordered().title("Participant name"));
    let mut error = String::new();
    loop {
        terminal.draw(|frame| {
            let [_, content, _] = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(7),
                Constraint::Fill(1),
            ])
            .areas(frame.area());
            let [_, content, _] = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Percentage(80),
                Constraint::Fill(1),
            ])
            .areas(content);
            let [title, field, help] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(3),
            ])
            .areas(content);
            frame.render_widget(Paragraph::new("Platypus Telemetry — new session"), title);
            frame.render_widget(&input, field);
            let message = if error.is_empty() {
                "Enter: start session · Esc / Ctrl+C: quit\nSpaces in the log filename become underscores."
            } else {
                &error
            };
            frame.render_widget(Paragraph::new(message), help);
        })?;
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if is_quit(key) {
                    return Ok(None);
                }
                if key.code == KeyCode::Enter {
                    let name = input.lines().join(" ");
                    match participant_file_name(&name) {
                        Ok(_) => return Ok(Some(name.trim().to_owned())),
                        Err(reason) => error = reason.to_string(),
                    }
                } else {
                    input.input(key);
                }
            }
            Event::Paste(text) => {
                input.insert_str(text.replace(['\r', '\n', '\t'], " "));
            }
            _ => {}
        }
    }
}

fn is_quit(key: KeyEvent) -> bool {
    key.code == KeyCode::Esc
        || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn is_submit(key: KeyEvent) -> bool {
    (key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT))
        || (key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL))
}

fn observation(text: &str) -> LogData {
    LogData {
        device: DeviceType::Observer,
        tag: "Observation".into(),
        data: text.replace("\r\n", " ").replace(['\r', '\n'], " ").into(),
    }
}

fn editor() -> TextArea<'static> {
    let mut input = TextArea::default();
    input.set_block(Block::bordered().title("Observations"));
    input.set_placeholder_text("Enter an observation…");
    input
}

fn draw_session(
    frame: &mut ratatui::Frame,
    participant: &str,
    logs: &VecDeque<String>,
    state: &mut ListState,
    input: &TextArea<'_>,
    status: &str,
) {
    let [content, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).areas(frame.area());
    let [log_area, observation_area] =
        Layout::vertical([Constraint::Ratio(2, 3), Constraint::Ratio(1, 3)]).areas(content);
    let width = usize::from(log_area.width.saturating_sub(2).max(1));
    let items = logs.iter().map(|log| log_item(log, width));
    let list = List::new(items)
        .block(Block::bordered().title(format!("Live logs — {participant}")))
        .highlight_style(Style::default().fg(Color::Cyan));
    frame.render_stateful_widget(list, log_area, state);
    frame.render_widget(input, observation_area);
    frame.render_widget(
        Paragraph::new(format!(
            "Shift+Enter / Ctrl+S: save · Enter: newline · PgUp/PgDn: logs · Esc / Ctrl+C: quit\n{status}"
        )),
        footer,
    );
}

fn log_item(log: &str, width: usize) -> ListItem<'static> {
    let mut display = String::with_capacity(log.len());
    for ch in log.chars() {
        match ch {
            '\t' => display.push_str("    "),
            '\n' => display.push('\n'),
            ch if ch.is_control() => display.extend(ch.escape_default()),
            ch => display.push(ch),
        }
    }
    let mut lines = Vec::new();
    for line in display.lines() {
        let mut wrapped = String::new();
        let mut used = 0;
        for ch in line.chars() {
            let char_width = ch.width().unwrap_or(0);
            if used + char_width > width && !wrapped.is_empty() {
                lines.push(Line::from(std::mem::take(&mut wrapped)));
                used = 0;
            }
            wrapped.push(ch);
            used += char_width;
        }
        lines.push(Line::from(wrapped));
    }
    ListItem::new(lines)
}

pub fn run(
    terminal: &mut DefaultTerminal,
    participant: &str,
    file: &Arc<Mutex<File>>,
    messages: &mpsc::Receiver<String>,
) -> io::Result<()> {
    let mut input = editor();
    let mut logs = VecDeque::from([format!("Participant: {participant} — connecting to MQTT…")]);
    let mut state = ListState::default();
    let mut following = true;
    let mut status = String::new();
    loop {
        for message in messages.try_iter().take(LOG_HISTORY) {
            if logs.len() == LOG_HISTORY {
                logs.pop_front();
                state.select(state.selected().map(|index| index.saturating_sub(1)));
            }
            logs.push_back(message);
        }
        if following {
            state.select(Some(logs.len().saturating_sub(1)));
        }
        terminal.draw(|frame| {
            draw_session(frame, participant, &logs, &mut state, &input, &status);
        })?;
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if is_quit(key) {
                    return Ok(());
                }
                if is_submit(key) {
                    let text = input.lines().join("\n");
                    if text.trim().is_empty() {
                        status = "Enter an observation before saving.".into();
                        continue;
                    }
                    let result = file
                        .lock()
                        .map_err(|_| io::Error::other("Log mutex poisoned"))
                        .and_then(|mut file| {
                            write_to_file(observation(&text), "observations", &mut *file)
                        });
                    match result {
                        Ok(log) => {
                            if logs.len() == LOG_HISTORY {
                                logs.pop_front();
                            }
                            logs.push_back(log.to_string());
                            input = editor();
                            following = true;
                            status = "Observation saved.".into();
                        }
                        Err(error) => status = format!("Failed to save observation: {error}"),
                    }
                } else if key.code == KeyCode::PageUp {
                    following = false;
                    state.select(Some(state.selected().unwrap_or(0).saturating_sub(5)));
                } else if key.code == KeyCode::PageDown {
                    let next =
                        (state.selected().unwrap_or(0) + 5).min(logs.len().saturating_sub(1));
                    state.select(Some(next));
                    following = next == logs.len().saturating_sub(1);
                } else {
                    input.input(key);
                }
            }
            Event::Paste(text) => {
                input.insert_str(text);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_shortcuts_do_not_intercept_plain_enter() {
        assert!(is_submit(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )));
        assert!(is_submit(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_submit(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )));
    }

    #[test]
    fn observations_inline_line_breaks_and_preserve_other_text() {
        let text = "  Participant hesitated.\nThen continued.\r\nPaused.\rFinished.  ";
        let log = observation(text);
        assert_eq!(log.device, DeviceType::Observer);
        assert_eq!(&*log.tag, "Observation");
        assert_eq!(
            &*log.data,
            "  Participant hesitated. Then continued. Paused. Finished.  "
        );
    }

    #[test]
    fn observations_and_telemetry_share_the_log_writer() {
        let mut file = io::Cursor::new(Vec::new());
        let text = "Participant paused\n\nthen continued.\r\nOh wow\rFinished.";
        write_to_file(observation(text), "observations", &mut file).unwrap();
        let telemetry = LogData {
            device: DeviceType::LillyGo,
            tag: "Vibrate".into(),
            data: "brimmm".into(),
        };
        write_to_file(telemetry, "test/data", &mut file).unwrap();
        let contents = String::from_utf8(file.into_inner()).unwrap();
        assert!(contents.contains(
            "|| Observer || observations || Observation ]:\tParticipant paused  then continued. Oh wow Finished.\n"
        ));
        assert!(contents.contains("|| LillyGo || test/data || Vibrate ]:\tbrimmm\n"));
        assert_eq!(contents.lines().count(), 2);
    }

    #[test]
    fn session_renders_logs_above_the_observation_editor() {
        let backend = ratatui::backend::TestBackend::new(100, 30);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let logs = VecDeque::from(["Telemetry received".to_owned()]);
        let mut state = ListState::default().with_selected(Some(0));
        terminal
            .draw(|frame| {
                draw_session(frame, "Jane Doe", &logs, &mut state, &editor(), "");
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        assert!(row(0).contains("Live logs — Jane Doe"));
        assert!(row(1).contains("Telemetry received"));
        assert!(row(19).contains("Observations"));
        assert!(row(28).contains("Shift+Enter / Ctrl+S"));
    }

    #[test]
    fn log_controls_are_rendered_as_text_not_terminal_commands() {
        let backend = ratatui::backend::TestBackend::new(100, 4);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let item = log_item("[ record ]:\t{\"increaseDiff\":5}\n\x1b[2J\r\0\x7f", 100);
                frame.render_widget(List::new([item]), frame.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        assert!(row(0).starts_with("[ record ]:    {\"increaseDiff\":5}"));
        assert!(row(1).starts_with("\\u{1b}[2J\\r\\u{0}\\u{7f}"));
        for cell in &buffer.content {
            assert!(!cell.symbol().chars().any(char::is_control));
        }
    }

    #[test]
    fn log_wrapping_counts_expanded_tabs() {
        let backend = ratatui::backend::TestBackend::new(8, 3);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(List::new([log_item("abc\tdef", 8)]), frame.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = |y| (0..8).map(|x| buffer[(x, y)].symbol()).collect::<String>();
        assert_eq!(row(0), "abc    d");
        assert_eq!(row(1), "ef      ");
    }

    #[test]
    fn tiny_terminals_do_not_panic() {
        for (width, height) in [(1, 1), (10, 4), (30, 10)] {
            let backend = ratatui::backend::TestBackend::new(width, height);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| {
                    draw_session(
                        frame,
                        "Jane Doe",
                        &VecDeque::from(["Observation: 你好\nNext line".to_owned()]),
                        &mut ListState::default().with_selected(Some(0)),
                        &editor(),
                        "",
                    );
                })
                .unwrap();
        }
    }
}
