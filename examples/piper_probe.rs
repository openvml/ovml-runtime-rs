use piper_rs::Piper;
use std::path::Path;

fn main() {
    let model = std::env::var("OVML_PIPER_PROBE_MODEL").unwrap_or_else(|_| "model.onnx".into());
    let config =
        std::env::var("OVML_PIPER_PROBE_CONFIG").unwrap_or_else(|_| format!("{model}.json"));
    match Piper::new(Path::new(&model), Path::new(&config)) {
        Ok(_) => println!("piper model loaded"),
        Err(err) => println!("piper model load failed: {err}"),
    }
}
