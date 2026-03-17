use anyhow::Result;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use std::sync::Mutex;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{HeapRb, traits::*};

use symphonia::core::{
    audio::AudioBufferRef,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};
use symphonia::core::audio::Signal;
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

    start_time: Arc<Mutex<Option<Instant>>>,
}

impl AudioEngine {
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();

        let playing = Arc::new(AtomicBool::new(false));
        let volume = Arc::new(AtomicU32::new(20));

        let start_time = Arc::new(Mutex::new(None));

        thread::spawn({
            let playing = playing.clone();
            let volume = volume.clone();
            let start_time = start_time.clone();

            move || run_audio_thread(rx, playing, volume, start_time)
        });

        Ok(Self {
            tx,
            playing,
            volume,
            start_time,
        })
    }

    // ===== API sesuai UI =====

    pub fn load(&self, path: &PathBuf) -> Result<()> {
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

    pub fn volume_up(&self) {
        let v = self.volume();
        let _ = self.tx.send(AudioCommand::Volume(v + 0.1));
    }

    pub fn volume_down(&self) {
        let v = self.volume();
        let _ = self.tx.send(AudioCommand::Volume(v - 0.1));
    }

    pub fn volume(&self) -> f32 {
        self.volume.load(Ordering::Relaxed) as f32 / 100.0
    }

    pub fn is_paused(&self) -> bool {
        !self.playing.load(Ordering::Relaxed)
    }

    pub fn elapsed(&self) -> Duration {
        if let Some(start) = *self.start_time.lock().unwrap() {
            start.elapsed()
        } else {
            Duration::ZERO
        }
    }

    pub fn finished(&self) -> bool {
        false // simple dulu
    }

    pub fn finalize_if_finished(&self) -> bool {
        false
    }
}

fn run_audio_thread(
    rx: mpsc::Receiver<AudioCommand>,
    playing: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
    start_time: Arc<Mutex<Option<Instant>>>,
) {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("no device");

    let config = device.default_output_config().unwrap();
    let sample_rate = config.sample_rate() as usize;
    let channels = config.channels() as usize;

    let rb = HeapRb::<f32>::new(sample_rate * channels * 2);
    let (producer, consumer) = rb.split();
    let producer = Arc::new(Mutex::new(producer));

    let cb_playing = playing.clone();
    let cb_volume = volume.clone();
    let mut cb_consumer = consumer;
    let started = Arc::new(AtomicBool::new(false));
    let cb_started = started.clone();

    let stream = device.build_output_stream(
        &config.into(),
        move |data: &mut [f32], _| {
            let vol = cb_volume.load(Ordering::Relaxed) as f32 / 100.0;
            let is_playing = cb_playing.load(Ordering::Relaxed);

            if !cb_started.load(Ordering::Relaxed) {
                if cb_consumer.occupied_len() < data.len() * 2 {
                    for s in data.iter_mut() {
                        *s = 0.0;
                    }
                    return;
                }
                cb_started.store(true, Ordering::Relaxed);
            }

            for sample in data.iter_mut() {
                if is_playing {
                    *sample = cb_consumer.try_pop().unwrap_or(0.0) * vol;
                } else {
                    *sample = 0.0;
                }
            }
        },
        move |err| eprintln!("audio error: {:?}", err),
        None,
    ).unwrap();

    stream.play().unwrap();

    let mut stop_flag: Option<Arc<AtomicBool>> = None;

    loop {
        if let Ok(cmd) = rx.recv() {
            match cmd {
                AudioCommand::Load(path) => {
                    if let Some(flag) = stop_flag.take() {
                        flag.store(true, Ordering::Relaxed);
                    }

                    let stop = Arc::new(AtomicBool::new(false));
                    stop_flag = Some(stop.clone());

                    let prod = producer.clone();

                    thread::spawn(move || {
                        decode_file(path, prod, stop);
                    });

                    *start_time.lock().unwrap() = Some(Instant::now());
                }

                AudioCommand::Play => {
                    playing.store(true, Ordering::Relaxed);
                }

                AudioCommand::Stop => {
                    playing.store(false, Ordering::Relaxed);
                }

                AudioCommand::Toggle => {
                    let v = !playing.load(Ordering::Relaxed);
                    playing.store(v, Ordering::Relaxed);
                }

                AudioCommand::Volume(v) => {
                    let v = (v.clamp(0.0, 2.0) * 100.0) as u32;
                    volume.store(v, Ordering::Relaxed);
                }
            }
        }
    }
}

fn decode_file(
    path: PathBuf,
    producer: Arc<Mutex<impl Producer<Item = f32>>>,
    stop: Arc<AtomicBool>,
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

    let track = match format.default_track() {
        Some(t) => t,
        None => return,
    };

    let mut decoder = match get_codecs().make(&track.codec_params, &Default::default()) {
        Ok(d) => d,
        Err(_) => return,
    };

    while !stop.load(Ordering::Relaxed) {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue,
        };

        match decoded {
            AudioBufferRef::F32(buf) => {
                let channels = buf.spec().channels.count();
                let frames = buf.frames();

                let mut p = producer.lock().unwrap();

                for frame in 0..frames {
                    for ch in 0..channels {
                        let sample = buf.chan(ch)[frame];
                        let _ = p.try_push(sample);
                    }
                }
            }

            AudioBufferRef::S16(buf) => {
                let channels = buf.spec().channels.count();
                let frames = buf.frames();

                let mut p = producer.lock().unwrap();

                for frame in 0..frames {
                    for ch in 0..channels {
                        let sample = buf.chan(ch)[frame] as f32 / i16::MAX as f32;
                        let _ = p.try_push(sample);
                    }
                }
            }

            _ => {}
        }
    }
}