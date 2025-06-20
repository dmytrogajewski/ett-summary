use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample};
use std::fs::File;
use std::io::BufWriter;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::time::Instant;

#[derive(Parser, Debug)]
#[command(version, about = "CPAL record from device", long_about = None)]
struct Opt {
    /// The audio device to use
    #[arg(short, long, default_value_t = String::from("default"))]
    device: String,

    /// Use the JACK host
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ),
        feature = "jack"
    ))]
    #[arg(short, long)]
    #[allow(dead_code)]
    jack: bool,
}

fn device() -> Result<cpal::Device, Box<dyn std::error::Error>> {
    let opt = Opt::parse();

    // Conditionally compile with jack if the feature is specified.
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ),
        feature = "jack"
    ))]
    // Manually check for flags. Can be passed through cargo with -- e.g.
    // cargo run --release --example beep --features jack -- --jack
    let host = if opt.jack {
        cpal::host_from_id(cpal::available_hosts()
            .into_iter()
            .find(|id| *id == cpal::HostId::Jack)
            .expect(
                "make sure --features jack is specified. only works on OSes where jack is available",
            )).expect("jack host unavailable")
    } else {
        cpal::default_host()
    };

    #[cfg(any(
        not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        )),
        not(feature = "jack")
    ))]
    let host = cpal::default_host();

    // Set up the input device and stream with the default input config.
    let device = if opt.device == "default" {
        host.default_output_device()
    } else {
        host.input_devices()?
            .find(|x| x.name().map(|y| y == opt.device).unwrap_or(false))
    }
    .expect("failed to find input device");

    Ok(device)
}

fn capture_audio<
    T: cpal::Sample + cpal::SizedSample + hound::Sample + std::marker::Send + 'static,
>(
    d: cpal::Device,
    cfg: cpal::StreamConfig,
    tx: mpsc::Sender<T>,
) -> Result<(), Box<dyn std::error::Error>> {
    let tx = Arc::new(Mutex::new(Some(tx)));

    let err_fn = |err| eprintln!("Stream error: {}", err);
    let writer_2 = tx.clone();
    let stream = d
        .build_input_stream(
            &cfg.into(),
            move |data: &[T], _: &_| write_input_data::<T, T>(data, &writer_2),
            err_fn,
            None,
        )
        .expect("Error building stream");

    stream.play()?;

    // Keep the stream running
    std::thread::sleep(std::time::Duration::from_secs(3600));
    drop(stream);
    Ok(())
}

fn capture_thread<T: cpal::SizedSample + hound::Sample + std::marker::Send + 'static>(
    d: cpal::Device,
    cfg: cpal::StreamConfig,
) -> mpsc::Receiver<T> {
    let (tx, rx) = mpsc::channel::<T>(44100 * 2 * 300);

    std::thread::spawn(move || {
        if let Err(e) = capture_audio(d, cfg, tx) {
            eprintln!("Error capturing audio: {}", e);
        }
    });

    rx
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let d = device().expect("Failed to get device");
    let cfg = d
        .default_input_config()
        .expect("Failed to get default input config");
    let spec = wav_spec_from_config(&cfg);

    println!("Default input config: {:?}", cfg);

    let strcfg: cpal::StreamConfig = cfg.clone().into();

    match cfg.sample_format() {
        cpal::SampleFormat::I8 => batch_and_send(capture_thread::<i8>(d, strcfg), spec).await?,
        cpal::SampleFormat::I16 => batch_and_send(capture_thread::<i16>(d, strcfg), spec).await?,
        cpal::SampleFormat::I32 => batch_and_send(capture_thread::<i32>(d, strcfg), spec).await?,
        cpal::SampleFormat::F32 => batch_and_send(capture_thread::<f32>(d, strcfg), spec).await?,
        _ => todo!(),
    }

    Ok(())
}

async fn batch_and_send<
    T: cpal::Sample + cpal::SizedSample + hound::Sample + std::marker::Send + 'static,
>(
    mut rx: mpsc::Receiver<T>,
    spec: hound::WavSpec,
) -> Result<(), Box<dyn std::error::Error>> {
    let samples_per_second = 44100;
    let channels: usize = 2;
    let total_samples = (samples_per_second * channels * 30) as u32; // 5 minutes
    let total_samples_u = total_samples as usize;
    let mut buffer: Vec<T> = Vec::with_capacity(total_samples_u);
    let start_time = Instant::now();

    while let Some(sample) = rx.recv().await {
        buffer.push(sample);

        if buffer.len() >= total_samples_u {
            let timestamp = start_time.elapsed().as_secs();
            let filename = format!("audio_{}.wav", timestamp);
            write_wav(&filename, &buffer, spec)?;

            // Send WAV to server
            send_wav(&filename).await?;

            // Clear buffer
            buffer.clear();
        }
    }

    Ok(())
}
fn sample_format(format: cpal::SampleFormat) -> hound::SampleFormat {
    if format.is_float() {
        hound::SampleFormat::Float
    } else {
        hound::SampleFormat::Int
    }
}
fn wav_spec_from_config(config: &cpal::SupportedStreamConfig) -> hound::WavSpec {
    hound::WavSpec {
        channels: config.channels() as _,
        sample_rate: config.sample_rate().0 as _,
        bits_per_sample: (config.sample_format().sample_size() * 8) as _,
        sample_format: sample_format(config.sample_format()),
    }
}

fn write_wav<T: hound::Sample + Clone>(
    filename: &str,
    samples: &[T],
    spec: hound::WavSpec,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = BufWriter::new(File::create(filename)?);
    let mut writer = hound::WavWriter::new(file, spec)?;
    for sample in samples {
        writer.write_sample(sample.clone())?;
    }
    writer.finalize()?;
    Ok(())
}

async fn send_wav(filename: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let file = tokio::fs::read(filename).await?;
    let part = reqwest::multipart::Part::bytes(file)
        .file_name(filename.to_string())
        .mime_str("audio/wav")?;

    let form = reqwest::multipart::Form::new().part("file", part);

    let response = client
        .post("http://localhost:8000/upload")
        .multipart(form)
        .send()
        .await?;

    if response.status().is_success() {
        println!("Successfully sent {}", filename);
    } else {
        eprintln!("Failed to send {}: {}", filename, response.status());
    }

    // Optionally delete the file after sending
    tokio::fs::remove_file(filename).await?;

    Ok(())
}
type WavWriterHandle<T> = Arc<Mutex<Option<mpsc::Sender<T>>>>;

fn write_input_data<T, U>(input: &[T], writer: &WavWriterHandle<U>)
where
    T: Sample,
    U: Sample + hound::Sample + FromSample<T>,
{
    if let Ok(mut guard) = writer.try_lock() {
        if let Some(writer) = guard.as_mut() {
            for &sample in input.iter() {
                let sample: U = U::from_sample(sample);
                writer.try_send(sample).ok();
            }
        }
    }
}
