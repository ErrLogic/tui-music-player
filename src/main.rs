mod audio;
mod playback;
mod track;
mod app;
mod ui;

use anyhow::Result;
use crossterm::{
    execute,
    terminal::{enable_raw_mode, disable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    event::{self, Event, KeyCode},
};
use tui::{Terminal, backend::CrosstermBackend};
use std::{
    io,
    time::{Duration, Instant}
};
use app::App;
use ui::{
    render::draw_ui,
    screens::Screen
};

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let music_dir = std::env::args().nth(1).unwrap_or("/home/agik/Songs".into());
    let mut app = App::new(music_dir.into())?;

    let tick_rate = Duration::from_millis(50);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| draw_ui(f, &mut app))?;

        // === INPUT ===
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout)? {
            if let Event::Key(k) = event::read()? {
                match k.code {
                    KeyCode::Char('q') => break,

                    KeyCode::Tab => {
                        app.ui_screen = match app.ui_screen {
                            Screen::TrackList => Screen::Player,
                            Screen::Player => Screen::TrackList,
                        }
                    }

                    KeyCode::Enter | KeyCode::Char(' ') => {
                        match app.ui_screen {
                            Screen::TrackList => {
                                app.play_selected()?;
                                app.ui_screen = Screen::Player;
                            }
                            Screen::Player => {
                                app.audio.toggle();
                            }
                        }
                    }

                    KeyCode::Down => {
                        app.selected = (app.selected + 1).min(app.tracks.len().saturating_sub(1));
                        app.list_state.select(Some(app.selected));
                    }

                    KeyCode::Up => {
                        app.selected = app.selected.saturating_sub(1);
                        app.list_state.select(Some(app.selected));
                    }

                    KeyCode::Right => {
                        app.selected = (app.playback.index + 1) % app.tracks.len();
                        app.play_selected()?;
                    }

                    KeyCode::Left => {
                        app.selected = if app.playback.index == 0 {
                            app.tracks.len() - 1
                        } else {
                            app.playback.index - 1
                        };
                        app.play_selected()?;
                    }

                    KeyCode::Char('+') | KeyCode::Char('=') => app.audio.volume_up(),
                    KeyCode::Char('-') => app.audio.volume_down(),

                    _ => {}
                }
            }
        }

        // === TICK ===
        if last_tick.elapsed() >= tick_rate {
            app.tick = app.tick.wrapping_add(1);
            app.auto_next()?; // kalau ada
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
