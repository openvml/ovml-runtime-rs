use kothok_edge_tts::{EdgeTts, Engine as _, TtsEvent};

#[tokio::main]
async fn main() {
    kothok_edge_tts::init_tls();
    let voice = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "en-US-AriaNeural".into());
    let text = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "Hello world.".into());
    let language = voice.split('-').take(2).collect::<Vec<_>>().join("-");
    match EdgeTts.synthesize(&text, &voice, "+0%", &language).await {
        Ok(events) => {
            let mut audio = 0usize;
            let mut words = 0usize;
            for event in events {
                match event {
                    TtsEvent::Audio(chunk) => audio += chunk.len(),
                    TtsEvent::WordBoundary { .. } => words += 1,
                    TtsEvent::TurnEnd => {}
                }
            }
            println!("audio_bytes={audio} words={words}");
        }
        Err(err) => println!("error={err}"),
    }
}
