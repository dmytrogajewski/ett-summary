use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    routing::post,
    Router,
};
use hound;
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::sync::Mutex;
use toml;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Clone, Deserialize)]
struct Config {
    openai_api_url: String,
    openai_model: String,
    webhook_url: String,
    webhook_template: String,
    whisper_model_path: String,
}

struct SharedState {
    summary: String,
    last_received: Instant,
    ctx: WhisperContext,
}

impl SharedState {
    fn new(model_path: &str) -> Self {
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .expect("failed to load model");
        Self {
            summary: String::new(),
            last_received: Instant::now(),
            ctx,
        }
    }
}

type StateHandle = Arc<Mutex<SharedState>>;

#[derive(Clone)]
struct AppState {
    shared: StateHandle,
    config: Arc<Config>,
    key: Arc<String>,
}

async fn load_config() -> Config {
    let path = env::var("CONFIG_FILE").unwrap_or_else(|_| "server-rs/config.toml".to_string());
    let text = fs::read_to_string(&path)
        .await
        .expect("failed to read config file");
    toml::from_str(&text).expect("invalid config")
}

async fn transcribe_wav(data: Vec<u8>, state: &mut SharedState) -> Result<String, String> {
    let cursor = std::io::Cursor::new(data);
    let mut reader = hound::WavReader::new(cursor).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != 16_000 {
        return Err("wav must be mono 16kHz".to_string());
    }
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .map(|s| s.unwrap_or_default())
        .collect();
    let mut float_samples = vec![0.0f32; samples.len()];
    whisper_rs::convert_integer_to_float_audio(&samples, &mut float_samples)
        .map_err(|e| e.to_string())?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    let mut wstate = state.ctx.create_state().map_err(|e| e.to_string())?;
    wstate
        .full(params, &float_samples[..])
        .map_err(|e| e.to_string())?;

    let num_segments = wstate.full_n_segments().map_err(|e| e.to_string())?;
    let mut text = String::new();
    for i in 0..num_segments {
        let seg = wstate.full_get_segment_text(i).map_err(|e| e.to_string())?;
        text.push_str(seg.trim());
        text.push(' ');
    }
    Ok(text)
}

async fn summarize_text(
    prompt: String,
    key: &str,
    api_url: &str,
    model: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}]
    });

    let res = client
        .post(api_url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !res.status().is_success() {
        return Err(format!("OpenAI error: {}", res.status()));
    }

    let resp_json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(resp_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string())
}

async fn post_webhook(url: &str, template: &str, summary: &str) {
    if url.is_empty() {
        return;
    }
    let payload = template.replace("{summary}", summary);
    let client = reqwest::Client::new();
    let _ = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await;
}

async fn upload_audio(State(app): State<AppState>, mut multipart: Multipart) -> StatusCode {
    let state = &app.shared;
    let cfg = &app.config;
    let key = &app.key;
    let mut data = None;
    while let Some(field) = multipart.next_field().await.unwrap() {
        if let Some(name) = field.name() {
            if name == "file" {
                data = Some(field.bytes().await.unwrap().to_vec());
                break;
            }
        }
    }

    let data = match data {
        Some(d) => d,
        None => return StatusCode::BAD_REQUEST,
    };

    let mut s = state.lock().await;
    s.last_received = Instant::now();
    let transcription = match transcribe_wav(data, &mut s).await {
        Ok(t) => t,
        Err(_) => return StatusCode::BAD_REQUEST,
    };

    let prompt = if s.summary.is_empty() {
        format!("Summarize this transcription: {}\n\nWith this template:\n### 🔴 Incident Summary\n\n**Time:** <START_TIME> – <END_TIME>  \n**Status:** <⚠️ Ongoing / ✅ Resolved / 🕓 Monitoring>  \n**Severity:** <SEV-1 / SEV-2 / SEV-3>  \n**Detected By:** <Monitoring / User Reports / Other>\n\n**Impact:**  \n<Brief description of the impact. Who/what was affected? Include user-facing symptoms, affected services, environments, or regions.>\n\n**Root Cause (Preliminary):**  \n<Short technical description of what caused the incident.>\n\n**Actions Taken:**  \n- <[TIMESTAMP]> <Action #1>  \n- <[TIMESTAMP]> <Action #2>  \n- …\n\n**Next Steps / Preventative Actions:**  \n- [ ] <Fix or mitigation #1>  \n- [ ] <Fix or mitigation #2>  \n- …\n\n**Owner:** <@team-or-person>  \n**Links:**  \n- Dashboard: <link>  \n- Logs: <link>  \n- Incident Tracker: <link>  \n- Postmortem: <link or TBD>", transcription)
    } else {
        format!("Here is text summary:\n{}\nPlease update this summary with new infroamtion from this transcription:\n{}", s.summary, transcription)
    };

    drop(s);
    let mut s = state.lock().await;
    match summarize_text(prompt, &key, &cfg.openai_api_url, &cfg.openai_model).await {
        Ok(sum) => {
            s.summary = sum.clone();
            post_webhook(&cfg.webhook_url, &cfg.webhook_template, &sum).await;
            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn flush_task(state: StateHandle) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        let mut s = state.lock().await;
        if !s.summary.is_empty()
            && Instant::now().duration_since(s.last_received) >= Duration::from_secs(3600)
        {
            s.summary.clear();
        }
    }
}

#[tokio::main]
async fn main() {
    let key = Arc::new(env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set"));
    let config = load_config().await;
    let state = Arc::new(Mutex::new(SharedState::new(&config.whisper_model_path)));

    let state_bg = state.clone();
    tokio::spawn(flush_task(state_bg));

    let app_state = AppState {
        shared: state.clone(),
        config: Arc::new(config.clone()),
        key: key.clone(),
    };

    let app = Router::new()
        .route("/upload", post(upload_audio))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8000));
    println!("Listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
