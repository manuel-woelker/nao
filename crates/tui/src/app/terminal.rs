use super::App;
use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use nao_base::result::NaoResult;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout};
use std::time::Duration;

impl App {
    /// Starts the full-screen terminal event loop.
    pub fn run(&mut self) -> NaoResult<()> {
        let mut terminal = TerminalSession::enter()?;
        loop {
            self.refresh()?;
            terminal.terminal.draw(|frame| self.render(frame))?;
            if event::poll(Duration::from_millis(150))?
                && let Event::Key(key_event) = event::read()?
                && self.handle_key_event(key_event)?
            {
                break;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn enter() -> NaoResult<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}
