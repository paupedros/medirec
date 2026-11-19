use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// whisper.cpp scales badly past ~half the logical cores (hybrid E-cores, SMT),
/// and using fewer leaves the PC responsive.
fn threads() -> i32 {
    if let Some(n) = std::env::var("MEDIREC_THREADS").ok().and_then(|v| v.parse().ok()) {
        return n;
    }
    let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (logical / 2).clamp(1, 6) as i32
}

/// Load the model once; reuse the context for every transcription.
pub fn load(model: &Path) -> Result<WhisperContext, String> {
    if !model.exists() {
        return Err(format!("Falta el modelo de voz: {}", model.display()));
    }
    // flash_attn measured slower on CPU (7.8 s vs 6.3 s on 21 s audio), so leave it off.
    WhisperContext::new_with_params(
        model.to_str().ok_or("Ruta de modelo inválida")?,
        WhisperContextParameters::default(),
    )
        .map_err(|e| format!("No se pudo cargar el modelo: {e}"))
}

pub fn run(
    ctx: &WhisperContext,
    samples: &[f32],
    on_progress: impl FnMut(i32) + 'static,
) -> Result<String, String> {
    let mut state = ctx.create_state().map_err(|e| e.to_string())?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("es"));
    params.set_translate(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_special(false);
    params.set_print_timestamps(false);
    params.set_n_threads(threads());
    params.set_progress_callback_safe(on_progress);

    state.full(params, samples).map_err(|e| format!("Error al transcribir: {e}"))?;

    let mut text = String::new();
    for seg in state.as_iter() {
        let s = seg.to_str_lossy().map_err(|e| e.to_string())?;
        let s = s.trim();
        if is_annotation(s) {
            continue;
        }
        text.push_str(s);
        text.push(' ');
    }
    Ok(text.trim().to_string())
}

/// Whisper labels silence/noise as "[Música]", "(aplausos)", etc. Drop those segments.
fn is_annotation(s: &str) -> bool {
    s.is_empty()
        || (s.starts_with('[') && s.ends_with(']'))
        || (s.starts_with('(') && s.ends_with(')'))
}

#[cfg(test)]
mod tests {
    #[test]
    fn drops_whisper_noise_labels_only() {
        assert!(super::is_annotation("[Música]"));
        assert!(super::is_annotation("(aplausos)"));
        assert!(super::is_annotation(""));
        assert!(!super::is_annotation("Toma 1 comprimido (500 mg) al día."));
    }
}
