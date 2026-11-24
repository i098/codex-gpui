use super::{FRAME_SAMPLES, SAMPLE_RATE};
use anyhow::{Context as _, Result, ensure};
use async_channel::{Receiver, Sender};
use cpal::{
    FromSample, Sample, SampleFormat, Stream,
    traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _},
};
use ringbuf::{
    HeapProd, HeapRb,
    traits::{Consumer as _, Producer as _, Split as _},
};

pub(super) type Frame = [f32; FRAME_SAMPLES];

pub(super) struct Speaker {
    samples: HeapProd<f32>,
    rate: Rate,
}

impl Speaker {
    pub(super) fn push(&mut self, input: &[f32]) {
        for &sample in input {
            self.rate.convert(sample, |sample| {
                let _ = self.samples.try_push(sample);
            });
        }
    }
}

struct Rate {
    input: u32,
    output: u32,
    phase: u32,
}

impl Rate {
    fn new(input: u32, output: u32) -> Self {
        Self {
            input,
            output,
            phase: 0,
        }
    }

    fn convert(&mut self, sample: f32, mut write: impl FnMut(f32)) {
        self.phase += self.output;
        while self.phase >= self.input {
            self.phase -= self.input;
            write(sample);
        }
    }
}

struct Capture {
    frames: Sender<Frame>,
    frame: Frame,
    filled: usize,
    channels: usize,
    rate: Rate,
}

impl Capture {
    fn copy<T: Sample>(&mut self, input: &[T])
    where
        f32: FromSample<T>,
    {
        for channels in input.chunks_exact(self.channels) {
            let mono = channels
                .iter()
                .map(|sample| sample.to_sample::<f32>())
                .sum::<f32>()
                / self.channels as f32;
            self.rate.convert(mono, |sample| {
                self.frame[self.filled] = sample;
                self.filled += 1;
                if self.filled == FRAME_SAMPLES {
                    let _ = self.frames.try_send(self.frame);
                    self.filled = 0;
                }
            });
        }
    }
}

struct Playback {
    samples: ringbuf::HeapCons<f32>,
    channels: usize,
}

impl Playback {
    fn copy<T: Sample + FromSample<f32>>(&mut self, output: &mut [T]) {
        for channels in output.chunks_exact_mut(self.channels) {
            channels.fill(self.samples.try_pop().unwrap_or(0.).to_sample::<T>());
        }
    }
}

pub(super) struct Devices {
    pub(super) microphone: Receiver<Frame>,
    pub(super) speaker: Speaker,
    pub(super) errors: Receiver<cpal::Error>,
    _input: Stream,
    _output: Stream,
}

fn supported(format: SampleFormat) -> bool {
    matches!(
        format,
        SampleFormat::F32
            | SampleFormat::I16
            | SampleFormat::I24
            | SampleFormat::I32
            | SampleFormat::U16
            | SampleFormat::F64
    )
}

pub(super) fn open() -> Result<Devices> {
    let host = cpal::default_host();
    let microphone = host
        .default_input_device()
        .context("No microphone for voice chat")?;
    let speaker = host
        .default_output_device()
        .context("No speaker for voice chat")?;
    let input = microphone.default_input_config()?;
    let output = speaker.default_output_config()?;
    ensure!(
        supported(input.sample_format()) && supported(output.sample_format()),
        "Unsupported voice audio format"
    );
    let (frames, captured) = async_channel::bounded(5);
    let (error, errors) = async_channel::bounded(1);
    let mut capture = Capture {
        frames,
        frame: [0.; FRAME_SAMPLES],
        filled: 0,
        channels: input.channels() as usize,
        rate: Rate::new(input.sample_rate(), SAMPLE_RATE),
    };
    let (samples, playback) = HeapRb::new(output.sample_rate() as usize / 5).split();
    let mut playback = Playback {
        samples: playback,
        channels: output.channels() as usize,
    };
    let input_error = error.clone();
    let input_stream = microphone
        .build_input_stream_raw(
            input.config(),
            input.sample_format(),
            move |data, _| match data.sample_format() {
                SampleFormat::F32 => capture.copy(data.as_slice::<f32>().unwrap()),
                SampleFormat::I16 => capture.copy(data.as_slice::<i16>().unwrap()),
                SampleFormat::I24 => capture.copy(data.as_slice::<cpal::I24>().unwrap()),
                SampleFormat::I32 => capture.copy(data.as_slice::<i32>().unwrap()),
                SampleFormat::U16 => capture.copy(data.as_slice::<u16>().unwrap()),
                SampleFormat::F64 => capture.copy(data.as_slice::<f64>().unwrap()),
                _ => {}
            },
            move |error| {
                let _ = input_error.try_send(error);
            },
            None,
        )
        .context("Unable to open microphone for voice chat")?;
    let output_stream = speaker
        .build_output_stream_raw(
            output.config(),
            output.sample_format(),
            move |data, _| match data.sample_format() {
                SampleFormat::F32 => playback.copy(data.as_slice_mut::<f32>().unwrap()),
                SampleFormat::I16 => playback.copy(data.as_slice_mut::<i16>().unwrap()),
                SampleFormat::I24 => playback.copy(data.as_slice_mut::<cpal::I24>().unwrap()),
                SampleFormat::I32 => playback.copy(data.as_slice_mut::<i32>().unwrap()),
                SampleFormat::U16 => playback.copy(data.as_slice_mut::<u16>().unwrap()),
                SampleFormat::F64 => playback.copy(data.as_slice_mut::<f64>().unwrap()),
                _ => {}
            },
            move |failure| {
                let _ = error.try_send(failure);
            },
            None,
        )
        .context("Unable to open speaker for voice chat")?;
    output_stream.play()?;
    input_stream.play()?;
    Ok(Devices {
        microphone: captured,
        speaker: Speaker {
            samples,
            rate: Rate::new(SAMPLE_RATE, output.sample_rate()),
        },
        errors,
        _input: input_stream,
        _output: output_stream,
    })
}
