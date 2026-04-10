use std::io::{stdout, Stdout};
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    execute,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use crate::executive::error::{Result, FcpError};

pub fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().map_err(|e| FcpError::Config(format!("Failed to enable raw mode: {}", e)))?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)
        .map_err(|e| FcpError::Config(format!("Failed to enter alt screen: {}", e)))?;

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        let _ = restore_terminal();
        original_hook(panic);
    }));

    Terminal::new(CrosstermBackend::new(stdout))
        .map_err(|e| FcpError::Config(format!("Failed to init terminal: {}", e)))
}

pub fn restore_terminal() -> Result<()> {
    disable_raw_mode().map_err(|e| FcpError::Config(format!("Failed to disable raw mode: {}", e)))?;
    execute!(stdout(), LeaveAlternateScreen)
        .map_err(|e| FcpError::Config(format!("Failed to leave alt screen: {}", e)))?;
    Ok(())
}
