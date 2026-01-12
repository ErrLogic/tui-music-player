use anyhow::Result;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::{
    fs::File,
    io::BufReader,
    path::PathBuf,
    time::{Duration, Instant},
};

pub struct AudioEngine {
    sink: Option<Sink>,
    _stream: OutputStream,
    handle: OutputStreamHandle,

    is_playing: bool,
    volume: f32,

    started_at: Option<Instant>,
    paused_at: Duration,
}

impl AudioEngine {
    pub fn new() -> Result<Self> {
        let (stream, handle) = OutputStream::try_default()?;
        Ok(Self {
            sink: None,
            _stream: stream,
            handle,
            is_playing: false,
            volume: 0.2,
            started_at: None,
            paused_at: Duration::ZERO,
        })
    }

    pub fn load(&mut self, path: &PathBuf) -> Result<()> {
        self.stop();

        let file = BufReader::new(File::open(path)?);
        let source = Decoder::new(file)?;

        let sink = Sink::try_new(&self.handle)?;
        sink.append(source);
        sink.pause();
        sink.set_volume(self.volume);

        self.sink = Some(sink);
        self.started_at = None;
        self.paused_at = Duration::ZERO;
        self.is_playing = false;

        Ok(())
    }

    pub fn play(&mut self) {
        if let Some(sink) = &self.sink {
            if !self.is_playing {
                sink.play();
                self.started_at = Some(Instant::now());
                self.is_playing = true;
            }
        }
    }

    pub fn pause(&mut self) {
        if let Some(sink) = &self.sink {
            if self.is_playing {
                sink.pause();
                if let Some(start) = self.started_at {
                    self.paused_at += start.elapsed();
                }
                self.started_at = None;
                self.is_playing = false;
            }
        }
    }

    pub fn toggle(&mut self) {
        if self.is_playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn stop(&mut self) {
        if let Some(sink) = &self.sink {
            sink.stop();
        }
        self.sink = None;
        self.started_at = None;
        self.paused_at = Duration::ZERO;
        self.is_playing = false;
    }

    // ===== READ-ONLY API (for UI) =====

    pub fn is_paused(&self) -> bool {
        !self.is_playing
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn elapsed(&self) -> Duration {
        match (self.is_playing, self.started_at) {
            (true, Some(start)) => self.paused_at + start.elapsed(),
            _ => self.paused_at,
        }
    }

    pub fn finished(&self) -> bool {
        self.sink.as_ref().map(|s| s.empty()).unwrap_or(false)
    }

    pub fn finalize_if_finished(&mut self) -> bool {
        if let Some(s) = &self.sink {
            if s.empty() {
                self.is_playing = false;
                self.started_at = None;
                return true;
            }
        }
        false
    }

    // ===== COMMAND API =====

    pub fn volume_up(&mut self) {
        self.set_volume(self.volume + 0.1);
    }

    pub fn volume_down(&mut self) {
        self.set_volume(self.volume - 0.1);
    }

    fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 2.0);
        if let Some(s) = &self.sink {
            s.set_volume(self.volume);
        }
    }
}
