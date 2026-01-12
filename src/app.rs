use anyhow::Result;
use walkdir::WalkDir;
use std::path::PathBuf;
use tui::widgets::ListState;
use crate::{track::Track, audio::AudioEngine, playback::PlaybackState};
use crate::ui::screens::Screen;

pub struct App {
    pub tracks: Vec<Track>,
    pub audio: AudioEngine,
    pub playback: PlaybackState,

    pub ui_screen: Screen,
    pub selected: usize,

    pub tick: u64,
    pub list_state: ListState,
}

impl App {
    pub fn new(dir: PathBuf) -> Result<Self> {
        let mut tracks = Vec::new();

        for e in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            if e.path().is_file() {
                let ext = e.path().extension().and_then(|s| s.to_str()).unwrap_or("");
                if ["mp3", "wav", "flac", "ogg"].contains(&ext) {
                    tracks.push(Track::new(e.path().to_path_buf()));
                }
            }
        }

        tracks.sort_by(|a, b| a.title.cmp(&b.title));

        let mut list_state = ListState::default();
        list_state.select(Some(0));

        Ok(Self {
            audio: AudioEngine::new()?,
            playback: PlaybackState::new(),
            ui_screen: Screen::TrackList,
            selected: 0,
            tracks,
            tick: 0,
            list_state,
        })
    }

    pub fn play_selected(&mut self) -> Result<()> {
        self.playback.index = self.selected; 
        let track = &self.tracks[self.selected];
        self.audio.stop();
        self.audio.load(&track.path)?;
        self.audio.play();
        self.playback.duration = track.duration;
        Ok(())
    }

    pub fn auto_next(&mut self) -> Result<()> {
        if self.audio.finalize_if_finished() {
            let next = (self.playback.index + 1) % self.tracks.len();
            self.selected = next;
            self.play_selected()?;
        }
        Ok(())
    }
}
