use std::{path::PathBuf, time::Duration};

use symphonia::core::{
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

pub struct Track {
    pub title: String,
    pub path: PathBuf,
    pub duration: Duration,
}

impl Track {
    pub fn new(path: PathBuf) -> Self {
        let title = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let duration = get_audio_duration(&path).unwrap_or(Duration::ZERO);

        Self {
            title,
            path,
            duration,
        }
    }
}

fn get_audio_duration(path: &PathBuf) -> Option<Duration> {
    let file = std::fs::File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension() {
        hint.with_extension(ext.to_string_lossy().as_ref());
    }

    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ).ok()?;

    let track = probed.format.default_track()?;
    let params = track.codec_params.clone();

    match (params.n_frames, params.sample_rate) {
        (Some(frames), Some(rate)) => {
            Some(Duration::from_secs_f64(frames as f64 / rate as f64))
        }
        _ => None,
    }
}
