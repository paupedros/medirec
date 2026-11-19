#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod pdf;
mod transcribe;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use whisper_rs::WhisperContext;

#[derive(Default)]
struct AppState {
    recorder: Mutex<Option<audio::Recorder>>,
    /// Arc so transcription can share it without copying the whole audio.
    samples: Mutex<Arc<Vec<f32>>>,
    /// Loaded once (preloaded at startup), reused for every transcription.
    model: Mutex<Option<Arc<WhisperContext>>>,
}

/// Returns the cached model, loading it if needed. Blocks; call off the UI thread.
fn model(app: &tauri::AppHandle) -> Result<Arc<WhisperContext>, String> {
    let state = app.state::<AppState>();
    let mut slot = state.model.lock().unwrap();
    if let Some(m) = slot.as_ref() {
        return Ok(m.clone());
    }
    let m = Arc::new(transcribe::load(&model_path(app))?);
    *slot = Some(m.clone());
    Ok(m)
}

fn secs(n: usize) -> u32 {
    (n as u32) / audio::TARGET_RATE
}

#[tauri::command]
fn start_recording(state: State<AppState>, device: Option<String>) -> Result<(), String> {
    let mut r = state.recorder.lock().unwrap();
    if r.is_none() {
        *r = Some(audio::Recorder::start(device)?);
    }
    Ok(())
}

#[tauri::command]
fn list_input_devices() -> (Vec<String>, Option<String>) {
    audio::list_inputs()
}

/// Raw WAV bytes of the current audio, for the in-app player.
#[tauri::command]
fn get_audio_wav(state: State<AppState>) -> tauri::ipc::Response {
    tauri::ipc::Response::new(audio::to_wav(&state.samples.lock().unwrap()))
}

/// (level 0..1000, seconds recorded excluding pauses)
#[tauri::command]
fn rec_status(state: State<AppState>) -> (u32, f32) {
    state
        .recorder
        .lock()
        .unwrap()
        .as_ref()
        .map(|r| (r.level.load(Ordering::Relaxed), r.seconds()))
        .unwrap_or((0, 0.0))
}

#[tauri::command]
fn pause_recording(state: State<AppState>, paused: bool) {
    if let Some(r) = state.recorder.lock().unwrap().as_ref() {
        r.paused.store(paused, Ordering::Relaxed);
    }
}

/// Stops and throws the recording away.
#[tauri::command]
fn cancel_recording(state: State<AppState>) {
    state.recorder.lock().unwrap().take(); // Drop stops the stream
}

/// Returns (duration in seconds, true if almost silent).
#[tauri::command]
fn stop_recording(state: State<AppState>) -> Result<(u32, bool), String> {
    let rec = state.recorder.lock().unwrap().take().ok_or("No se estaba grabando.")?;
    let samples = rec.finish();
    let d = secs(samples.len());
    if d < 1 {
        return Err("La grabación es demasiado corta.".into());
    }
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    *state.samples.lock().unwrap() = Arc::new(samples);
    Ok((d, peak < 0.03))
}

/// Opens file dialog, decodes. Returns (file name, seconds) or None if cancelled.
#[tauri::command]
async fn pick_audio(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<(String, u32)>, String> {
    let Some(fp) = app
        .dialog()
        .file()
        .add_filter("Audio", &["mp3", "wav", "m4a", "mp4", "aac", "ogg", "flac"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let samples = tauri::async_runtime::spawn_blocking(move || audio::decode_file(&path))
        .await
        .map_err(|e| e.to_string())??;
    let d = secs(samples.len());
    *state.samples.lock().unwrap() = Arc::new(samples);
    Ok(Some((name, d)))
}

fn model_path(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .resource_dir()
        .map(|d| d.join("models").join("ggml-small-q5_1.bin"))
        .unwrap_or_default()
}

#[tauri::command]
async fn transcribe(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let samples = state.samples.lock().unwrap().clone(); // Arc clone, no copy
    if samples.is_empty() {
        return Err("Primero grabe o elija un audio.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let ctx = model(&app)?;
        transcribe::run(&ctx, &samples, move |p| {
            let _ = app.emit("progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Returns true if saved, false if cancelled.
#[tauri::command]
fn export_pdf(app: tauri::AppHandle, title: String, date: String, text: String) -> Result<bool, String> {
    let Some(fp) = app
        .dialog()
        .file()
        .add_filter("PDF", &["pdf"])
        .set_file_name("Transcripcion.pdf")
        .blocking_save_file()
    else {
        return Ok(false);
    };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    pdf::export(&path, &title, &date, &text)?;
    Ok(true)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            // 1000x740 fits every screen without scrolling. On small displays
            // (e.g. 1366x768) that would not fit, so open maximized instead.
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(Some(m)) = win.current_monitor() {
                    let area = m.work_area().size.to_logical::<f64>(m.scale_factor());
                    if area.height < 740.0 + 40.0 || area.width < 1000.0 {
                        let _ = win.maximize();
                    }
                }
                let _ = win.show(); // created hidden to avoid a visible resize
            }
            // Preload the model while the user is still recording.
            let h = app.handle().clone();
            std::thread::spawn(move || {
                let _ = model(&h);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_recording,
            list_input_devices,
            get_audio_wav,
            rec_status,
            pause_recording,
            cancel_recording,
            stop_recording,
            pick_audio,
            transcribe,
            export_pdf
        ])
        .run(tauri::generate_context!())
        .expect("error al iniciar MediRec");
}
