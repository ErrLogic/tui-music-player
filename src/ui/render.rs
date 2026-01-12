use tui::{
    Frame,
    backend::Backend,
    layout::{Layout, Constraint, Direction},
    widgets::{Block, Borders, Paragraph, List, ListItem, Gauge},
    style::{Style, Modifier},
    text::{Span, Spans},
    layout::{Alignment, Rect}
};

use std::f64::consts::PI;
use crate::app::App;
use crate::ui::screens::Screen;

pub fn draw_ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let size = f.size();
    let compact = size.width < 50 || size.height < 18;

    if compact {
        draw_mini_ui(f, app);
    } else {
        draw_full_ui(f, app);
    }
}

fn draw_full_ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),   // main content
            Constraint::Length(2), // footer
        ])
        .split(f.size());

    match app.ui_screen {
        Screen::TrackList => draw_list(f, app, chunks[0]),
        Screen::Player => draw_player(f, app, chunks[0]),
    }

    let footer = Paragraph::new(
        "Tab=Switch Screen  Space=Play/Pause  ←/→=Prev/Next  +/-=Volume  q=Quit"
    )
        .block(Block::default().borders(Borders::TOP));

    f.render_widget(footer, chunks[1]);
}


fn draw_mini_ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    let area = f.size();

    match app.ui_screen {
        Screen::TrackList => draw_list(f, app, area),
        Screen::Player => draw_compact_player(f, app, area),
    }
}

fn marquee(text: &str, width: usize, tick: u64) -> String {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    if len <= width {
        return text.to_string();
    }

    let speed = 4;
    let gap = 4;

    let cycle = len + gap;
    let offset = ((tick / speed) as usize) % cycle;

    let mut out = String::with_capacity(width);

    for i in 0..width {
        let idx = offset + i;
        if idx < len {
            out.push(chars[idx]);
        } else if idx < cycle {
            out.push(' ');
        } else {
            out.push(chars[(idx - cycle) % len]);
        }
    }

    out
}

fn draw_list<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let inner_width = area.width.saturating_sub(4) as usize;
    // 2 border + 2 prefix

    let items: Vec<ListItem> = app.tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let selected = i == app.selected;
            let prefix = if selected { "▶ " } else { "  " };

            let available = inner_width.saturating_sub(prefix.len());

            let title = if selected {
                marquee(&t.title, available, app.tick)
            } else {
                truncate(&t.title, available)
            };

            ListItem::new(format!("{}{}", prefix, title))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().title("Tracks").borders(Borders::ALL));

    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_player<B: Backend>(f: &mut Frame<B>, app: &App, area: Rect) {
    let track = &app.tracks[app.playback.index];

    let finished = app.audio.finished();
    let elapsed = if finished {
        app.playback.duration
    } else {
        app.audio.elapsed()
    };

    let dur = app.playback.duration;

    let progress = if dur.as_secs() > 0 {
        (elapsed.as_secs_f64() / dur.as_secs_f64()).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(5),
        ])
        .split(area);

    let status = if app.audio.is_paused() { "⏸ Paused" } else { "▶ Playing" };
    let vol_pct = (app.audio.volume().clamp(0.0, 2.0) * 50.0) as u16;

    let title_width = layout[0].width.saturating_sub(4) as usize;

    let title = marquee(
        &track.title,
        title_width,
        app.tick,
    );

    let header = Paragraph::new(vec![
        Spans::from(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Spans::from(vec![
            Span::styled(status, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw("    "),
            Span::raw(format!("Vol {:>3}%", vol_pct)),
        ]),
    ])
        .block(Block::default().title("Now Playing").borders(Borders::ALL));

    f.render_widget(header, layout[0]);

    let progress_bar = Gauge::default()
        .ratio(progress)
        .gauge_style(
            Style::default() 
                .fg(tui::style::Color::White) 
                .bg(tui::style::Color::Black) 
                .add_modifier(Modifier::BOLD), 
        )
        .label(format!(
            "{:02}:{:02} / {:02}:{:02}",
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60,
            dur.as_secs() / 60,
            dur.as_secs() % 60
        ))
        .block(Block::default().title("Progress").borders(Borders::ALL));

    f.render_widget(progress_bar, layout[1]);

    let waveform = fake_waveform(
        elapsed.as_millis() as u64,
        layout[2].width,
        layout[2].height,
        app.audio.is_paused(),
    );

    let wave = Paragraph::new(waveform)
        .block(Block::default().title("Animation").borders(Borders::ALL));

    f.render_widget(wave, layout[2]);
}

fn fake_waveform(
    tick: u64,
    width: u16,
    height: u16,
    paused: bool,
) -> Vec<Spans<'static>> {
    let usable_width = width.saturating_sub(2) as usize;
    let usable_height = height.saturating_sub(2) as usize;

    if usable_width == 0 || usable_height == 0 {
        return vec![];
    }

    let speed = 0.0006;
    let phase = if paused {
        0.0
    } else {
        tick as f64 * speed
    };

    let quantized_phase = (phase * 10.0).floor() / 10.0;
    let paused_factor = if paused { 0.0 } else { 1.0 };

    // Generate amplitude per column
    let mut amps = Vec::with_capacity(usable_width);
    for x in 0..usable_width {
        let t = x as f64 / usable_width as f64 * PI * 2.0;
        let v =
            ((t + quantized_phase).sin() * 0.6 +
                (t * 2.0 + quantized_phase * 0.7).sin() * 0.3 +
                (t * 5.0 + quantized_phase * 0.2).sin() * 0.1)
                * paused_factor;

        let norm = ((v + 1.0) / 2.0).clamp(0.0, 1.0);
        amps.push((norm * usable_height as f64) as usize);
    }

    // Render vertical bars (centered)
    let mid = usable_height / 2;
    let mut rows = Vec::new();

    for row in (0..usable_height).rev() {
        let mut line = String::with_capacity(usable_width);

        for &amp in &amps {
            let top = mid + amp / 2;
            let bottom = mid.saturating_sub(amp / 2);

            if row >= bottom && row <= top {
                line.push('.');
            } else {
                line.push(' ');
            }
        }

        rows.push(Spans::from(Span::raw(line)));
    }

    rows
}

fn draw_compact_player<B: Backend>(f: &mut Frame<B>, app: &App, area: Rect) {
    let track = &app.tracks[app.playback.index];

    // === TIME STATE (NO SIDE EFFECT) ===
    let elapsed = if app.audio.finished() {
        app.playback.duration
    } else {
        app.audio.elapsed()
    };

    let dur = app.playback.duration;

    let status = if app.audio.is_paused() { "⏸" } else { "▶" };

    let vol_percent = (app.audio.volume().clamp(0.0, 2.0) * 50.0) as u8;

    // === VERTICAL LAYOUT ===
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // title
            Constraint::Length(1), // info row
        ])
        .split(area);

    // === TITLE ===
    let title_width = area.width.saturating_sub(4) as usize;

    let title = marquee(
        &track.title,
        title_width,
        app.tick,
    );

    let title = Paragraph::new(
        Spans::from(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        ))
    )
        .block(Block::default().title("Now").borders(Borders::ALL));

    f.render_widget(title, chunks[0]);

    // === INFO ROW (TIME + VOLUME) ===
    let info_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(70),
            Constraint::Percentage(30),
        ])
        .split(chunks[1]);

    let time_text = format!(
        "{} {:02}:{:02}/{:02}:{:02}",
        status,
        elapsed.as_secs() / 60,
        elapsed.as_secs() % 60,
        dur.as_secs() / 60,
        dur.as_secs() % 60,
    );

    let time = Paragraph::new(time_text)
        .alignment(Alignment::Left);

    let vol = Paragraph::new(format!("Vol {}%", vol_percent))
        .alignment(Alignment::Right);

    f.render_widget(time, info_chunks[0]);
    f.render_widget(vol, info_chunks[1]);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}


