use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{traits::*, HeapRb};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
    thread,
    time::Duration,
};

use symphonia::core::audio::Signal;
use symphonia::core::{
    audio::AudioBufferRef,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};
use symphonia::default::{get_codecs, get_probe};

// ================= COMMAND =================

pub enum AudioCommand {
    Load(PathBuf),
    Play,
    Stop,
    Toggle,
    Volume(f32),
}

// ================= ENGINE =================

pub struct AudioEngine {
    tx: Sender<AudioCommand>,

    playing: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,

    samples_played: Arc<AtomicU64>,
    sample_rate: usize,
    channels: usize,

    finished_flag: Arc<AtomicBool>,
    buffer_empty_flag: Arc<AtomicBool>,
    underrun_count: Arc<AtomicU64>,
}

impl AudioEngine {
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();

        let playing = Arc::new(AtomicBool::new(false));
        let volume = Arc::new(AtomicU32::new(20));
        let samples_played = Arc::new(AtomicU64::new(0));

        let finished_flag = Arc::new(AtomicBool::new(false));
        let buffer_empty_flag = Arc::new(AtomicBool::new(true));
        let underrun_count = Arc::new(AtomicU64::new(0));

        let host = cpal::default_host();
        let device = host.default_output_device().expect("no device");

        let config = device.default_output_config()?;
        let sample_rate = config.sample_rate() as usize;
        let channels = config.channels() as usize;

        thread::spawn({
            let playing = playing.clone();
            let volume = volume.clone();
            let samples_played = samples_played.clone();
            let finished_flag_cb = finished_flag.clone();
            let buffer_empty_flag_cb = buffer_empty_flag.clone();
            let underrun_cb = underrun_count.clone();

            move || run_audio_thread(
                rx,
                playing,
                volume,
                samples_played,
                finished_flag_cb,
                buffer_empty_flag_cb,
                underrun_cb,
            )
        });

        Ok(Self {
            tx,
            playing,
            volume,
            samples_played,
            sample_rate,
            channels,
            finished_flag,
            buffer_empty_flag,
            underrun_count,
        })
    }

    pub fn is_paused(&self) -> bool {
        !self.playing.load(Ordering::Relaxed)
    }

    pub fn volume(&self) -> f32 {
        self.volume.load(Ordering::Relaxed) as f32 / 100.0
    }

    pub fn volume_up(&self) {
        let v = self.volume();
        let _ = self.tx.send(AudioCommand::Volume(v + 0.1));
    }

    pub fn volume_down(&self) {
        let v = self.volume();
        let _ = self.tx.send(AudioCommand::Volume(v - 0.1));
    }

    pub fn finished(&self) -> bool {
        self.finished_flag.load(Ordering::Relaxed)
            && self.buffer_empty_flag.load(Ordering::Relaxed)
    }

    pub fn load(&self, path: &PathBuf) -> Result<()> {
        self.samples_played.store(0, Ordering::Relaxed);
        self.finished_flag.store(false, Ordering::Relaxed);
        self.buffer_empty_flag.store(true, Ordering::Relaxed);
        self.underrun_count.store(0, Ordering::Relaxed);

        self.tx.send(AudioCommand::Load(path.clone()))?;
        Ok(())
    }

    pub fn play(&self) {
        let _ = self.tx.send(AudioCommand::Play);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(AudioCommand::Stop);
    }

    pub fn toggle(&self) {
        let _ = self.tx.send(AudioCommand::Toggle);
    }

    pub fn elapsed(&self) -> Duration {
        let samples = self.samples_played.load(Ordering::Relaxed);
        let frames = samples / self.channels as u64;
        let seconds = frames as f64 / self.sample_rate as f64;
        Duration::from_secs_f64(seconds)
    }

    pub fn finalize_if_finished(&self) -> bool {
        if self.finished_flag.load(Ordering::Relaxed)
            && self.buffer_empty_flag.load(Ordering::Relaxed)
        {
            self.playing.store(false, Ordering::Relaxed);
            return true;
        }
        false
    }

    pub fn underruns(&self) -> u64 {
        self.underrun_count.load(Ordering::Relaxed)
    }
}

// ================= AUDIO THREAD =================

fn run_audio_thread(
    rx: mpsc::Receiver<AudioCommand>,
    playing: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
    samples_played: Arc<AtomicU64>,
    finished_flag: Arc<AtomicBool>,
    buffer_empty_flag: Arc<AtomicBool>,
    underrun_count: Arc<AtomicU64>,
) {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("no device");

    let config = device.default_output_config().unwrap();
    let sample_rate = config.sample_rate() as usize;
    let channels = config.channels() as usize;

    let rb = HeapRb::<f32>::new(sample_rate * channels * 2);
    let (producer, mut consumer) = rb.split();

    let playing_cb = playing.clone();
    let volume_cb = volume.clone();
    let buffer_empty_flag_cb2 = buffer_empty_flag.clone();
    let underrun_cb2 = underrun_count.clone();
    let samples_played_cb = samples_played.clone();
    let reset_flag = Arc::new(AtomicBool::new(false));
    let reset_flag_cb = reset_flag.clone();

    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _| {
            if reset_flag_cb.swap(false, Ordering::Relaxed) {
                while consumer.try_pop().is_some() {}
            }

            let vol = volume_cb.load(Ordering::Relaxed) as f32 / 100.0;
            let is_playing = playing_cb.load(Ordering::Relaxed);

            let mut local_samples = 0u64;
            let mut local_underrun = 0u64;

            let is_empty = consumer.is_empty();
            buffer_empty_flag_cb2.store(is_empty, Ordering::Relaxed);

            for sample in data.iter_mut() {
                if is_playing {
                    match consumer.try_pop() {
                        Some(s) => *sample = s * vol,
                        None => {
                            *sample = 0.0;
                            local_underrun += 1;
                        }
                    }
                    local_samples += 1;
                } else {
                    *sample = 0.0;
                }
            }

            if local_samples > 0 {
                samples_played_cb.fetch_add(local_samples, Ordering::Relaxed);
            }

            if local_underrun > 0 {
                underrun_cb2.fetch_add(local_underrun, Ordering::Relaxed);
            }
        },
        move |err| eprintln!("audio error: {:?}", err),
        None,
    ).unwrap();

    stream.play().unwrap();

    let mut stop_flag: Option<Arc<AtomicBool>> = None;
    let (decode_tx, decode_rx) = mpsc::channel::<(PathBuf, Arc<AtomicBool>)>();

    let mut producer = producer; // move sekali

    thread::spawn({
        let finished_flag = finished_flag.clone();

        move || {
            while let Ok((path, stop)) = decode_rx.recv() {
                decode_file(
                    path,
                    &mut producer,
                    stop,
                    finished_flag.clone(),
                    sample_rate,
                    channels,
                );
            }
        }
    });

    loop {
        if let Ok(cmd) = rx.recv() {
            match cmd {
                AudioCommand::Load(path) => {
                    if let Some(flag) = stop_flag.take() {
                        flag.store(true, Ordering::Relaxed);
                    }

                    reset_flag.store(true, Ordering::Relaxed);

                    finished_flag.store(false, Ordering::Relaxed);
                    playing.store(true, Ordering::Relaxed);

                    let stop = Arc::new(AtomicBool::new(false));
                    stop_flag = Some(stop.clone());

                    let _ = decode_tx.send((path, stop));
                }

                AudioCommand::Play => playing.store(true, Ordering::Relaxed),
                AudioCommand::Stop => playing.store(false, Ordering::Relaxed),
                AudioCommand::Toggle => {
                    let v = !playing.load(Ordering::Relaxed);
                    playing.store(v, Ordering::Relaxed);
                }
                AudioCommand::Volume(v) => {
                    volume.store((v.clamp(0.0, 2.0) * 100.0) as u32, Ordering::Relaxed);
                }
            }
        }
    }
}

// ================= RESAMPLER =================

fn linear_resample(input: &[f32], ratio: f32, out: &mut Vec<f32>) {
    let len = input.len();
    if len < 2 {
        return;
    }

    let expected = (len as f32 * ratio) as usize;
    out.reserve(expected);

    let step = 1.0 / ratio;
    let mut pos = 0.0;

    while pos < (len - 1) as f32 {
        let i = pos as usize;
        let frac = pos - i as f32;

        let a = unsafe { *input.get_unchecked(i) };
        let b = unsafe { *input.get_unchecked(i + 1) };

        out.push(a + (b - a) * frac);

        pos += step;
    }
}

// ================= DECODER =================

fn decode_file(
    path: PathBuf,
    producer: &mut impl Producer<Item = f32>,
    stop: Arc<AtomicBool>,
    finished_flag: Arc<AtomicBool>,
    device_sr: usize,
    _device_ch: usize,
) {
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension() {
        hint.with_extension(&ext.to_string_lossy());
    }

    let probed = match get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) {
        Ok(p) => p,
        Err(_) => return,
    };

    let mut format = probed.format;
    let track = format.default_track().unwrap();

    let src_sr = track.codec_params.sample_rate.unwrap_or(device_sr as u32) as usize;

    let mut decoder = get_codecs()
        .make(&track.codec_params, &Default::default())
        .unwrap();

    let ratio = device_sr as f32 / src_sr as f32;

    let mut resampled_channels: Vec<Vec<f32>> = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue,
        };

        if let AudioBufferRef::F32(buf) = decoded {
            let ch = buf.spec().channels.count();
            let frames = buf.frames();

            // init sekali, bukan tiap loop
            if resampled_channels.len() != ch {
                resampled_channels.clear();
                resampled_channels.resize_with(ch, || Vec::with_capacity(frames * 2));
            }

            for c in 0..ch {
                let input = &buf.chan(c)[..frames];
                let out = &mut resampled_channels[c];

                out.clear();
                out.reserve(input.len());

                linear_resample(input, ratio, out);
            }

            let min_len = resampled_channels
                .iter()
                .map(|v| v.len())
                .min()
                .unwrap_or(0);

            for i in 0..min_len {
                for c in 0..ch {
                    let s = resampled_channels[c][i];

                    if producer.try_push(s).is_err() {
                        continue; // drop
                    }
                }
            }
        }
    }

    finished_flag.store(true, Ordering::Relaxed);
}