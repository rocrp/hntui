use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{stdout, Stdout};
use std::panic;

pub type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct Tui {
    terminal: TuiTerminal,
}

impl Tui {
    pub fn init() -> Result<Self> {
        claim_terminal()?;

        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend).context("create terminal")?;
        terminal.clear().context("clear terminal")?;

        install_panic_hook();

        Ok(Self { terminal })
    }

    pub fn draw<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut ratatui::Frame),
    {
        self.terminal.draw(f).context("draw frame")?;
        Ok(())
    }

    /// Hands the terminal back to the shell so an external program can own it.
    ///
    /// The caller must also drop the crossterm event stream first: its reader
    /// thread sits in a blocking read on the tty and would swallow keystrokes
    /// meant for the editor.
    pub fn suspend(&mut self) -> Result<()> {
        // Every frame hides the cursor, and leaving the alternate screen does
        // not bring it back: cursor visibility is a terminal-global mode. An
        // editor that draws its own cursor masks this; `ed` does not.
        self.terminal
            .show_cursor()
            .context("show cursor before handing over the terminal")?;
        restore_terminal()
    }

    /// Takes the terminal back after an external program returns, redrawing
    /// from scratch since the screen and its size may both have changed.
    pub fn resume(&mut self) -> Result<()> {
        claim_terminal()?;
        self.terminal
            .clear()
            .context("clear terminal after resume")?;
        Ok(())
    }

    pub fn area(&self) -> Result<ratatui::layout::Rect> {
        let size = self.terminal.size().context("read terminal size")?;
        Ok(ratatui::layout::Rect::new(0, 0, size.width, size.height))
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = restore_terminal();
    }
}

fn claim_terminal() -> Result<()> {
    enable_raw_mode().context("enable raw mode")?;

    let mut out = stdout();
    out.execute(EnterAlternateScreen)
        .context("enter alternate screen")?;
    out.execute(EnableMouseCapture)
        .context("enable mouse capture")?;
    Ok(())
}

fn restore_terminal() -> Result<()> {
    disable_raw_mode().context("disable raw mode")?;

    let mut out = stdout();
    out.execute(DisableMouseCapture)
        .context("disable mouse capture")?;
    out.execute(LeaveAlternateScreen)
        .context("leave alternate screen")?;
    Ok(())
}

fn install_panic_hook() {
    let prev = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        prev(info);
    }));
}
