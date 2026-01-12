use std::time::Duration;

pub struct PlaybackState {
    pub index: usize,
    pub duration: Duration,
}

impl PlaybackState {
    pub fn new() -> Self {
        Self {
            index: 0,
            duration: Duration::ZERO,
        }
    }
}
