#[path = "../src/audio.rs"]
mod audio;
#[path = "../src/transcribe.rs"]
mod transcribe;

use std::path::Path;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (names, def) = audio::list_inputs();
    println!("Micrófonos: {names:?}\nPredeterminado: {def:?}");

    // 1) Record 2 s from each mic, report peak
    for name in &names {
        match audio::Recorder::start(Some(name.clone())) {
            Ok(rec) => {
                std::thread::sleep(std::time::Duration::from_secs(2));
                let s = rec.finish();
                let peak = s.iter().fold(0f32, |m, x| m.max(x.abs()));
                println!("{name}: {:.1} s, pico {:.4}", s.len() as f32 / 16000.0, peak);
            }
            Err(e) => println!("{name}: ERROR {e}"),
        }
    }

    // 2) Transcribe a file, with timing
    if let (Some(file), Some(model)) = (args.get(1), args.get(2)) {
        let t = Instant::now();
        let samples = audio::decode_file(Path::new(file)).expect("decode");
        println!("Audio: {:.1} s (decodificado en {:?})", samples.len() as f32 / 16000.0, t.elapsed());
        let t = Instant::now();
        let ctx = transcribe::load(Path::new(model)).expect("load");
        println!("Modelo cargado en {:?}", t.elapsed());
        for run in 1..=2 {
            let t = Instant::now();
            let text = transcribe::run(&ctx, &samples, |_| {}).expect("transcribe");
            println!("Transcripción {run} en {:?}", t.elapsed());
            if run == 2 {
                println!("{text}");
            }
        }
    }
}
