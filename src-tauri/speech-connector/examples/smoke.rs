//! Opt-in real-model regression. Does not download weights or modify a deck.
use sha2::{Digest, Sha256};
use speech_connector::{audio, Cancellation, QwenConnector, SpeechProvider, SynthesisRequest};
use std::{fs, path::PathBuf, sync::Arc};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("Usage: smoke ROOT HELPER NEW_SPOOL".into());
    }
    let spool = PathBuf::from(&args[2]);
    fs::create_dir(&spool)?;
    let provider = QwenConnector::new(PathBuf::from(&args[0]), Some(PathBuf::from(&args[1])));
    let english="Good work needs space. This presentation shows three simple ways to reduce interruptions, protect your attention, and make room for better ideas.";
    let german="Gute Arbeit braucht Ruhe. In dieser Präsentation zeigen wir drei einfache Wege, Unterbrechungen zu reduzieren, die eigene Aufmerksamkeit zu schützen und Raum für bessere Ideen zu schaffen.";
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let mut results=Vec::new();
        let cases=[("english",english,"en",1.0,"d0771e6412bdb48a0332be0e5b5fad473165af94450b5c2b6bb6488495312791"),("german",german,"de",1.1,"47153240b0e142ccf993d41eba1e6b0643936075916eb42331058623de4d4f56"),("english-paced",english,"en",1.1,"1f93661a14729caf340a49d6927ec4f582b0bf296a5bf508d8454ccc3918b50c")];
        for (name,text,language,pace,expected) in cases {
            let artifact=provider.synthesize(SynthesisRequest{text:text.into(),language:language.into(),voice_id:"preset:ryan".into(),pace},&spool,Cancellation::default(),Arc::new(|_,_,_,detail|eprintln!("{detail}"))).await?;
            let bytes=fs::read(&artifact.path)?;let hash=format!("{:x}",Sha256::digest(&bytes));assert_eq!(hash,expected,"Phase 2 regression: {name}");
            assert_eq!(audio::decode(&bytes)?.samples.len(),artifact.samples);
            results.push(serde_json::json!({"case":name,"seconds":artifact.samples as f64/artifact.sample_rate as f64,"sha256":hash}));
        }
        // Cancel after real inference progress, then reuse this same connector.
        let cancel=Cancellation::default();let trigger=cancel.clone();
        let failed=provider.synthesize(SynthesisRequest{text:german.into(),language:"de".into(),voice_id:"preset:ryan".into(),pace:1.1},&spool,cancel,Arc::new(move|_,_,_,detail|{if detail.contains("synthesized"){trigger.cancel();}})).await;
        assert!(failed.is_err());
        let artifact=provider.synthesize(SynthesisRequest{text:english.into(),language:"en".into(),voice_id:"preset:ryan".into(),pace:1.0},&spool,Cancellation::default(),Arc::new(|_,_,_,_|{})).await?;
        assert_eq!(format!("{:x}",Sha256::digest(fs::read(artifact.path)?)),cases[0].4);
        let result=serde_json::json!({"cases":results,"cancel_after_inference_and_restart":true,"standalone_without_tauri":true});fs::write(spool.join("results.json"),serde_json::to_vec_pretty(&result)?)?;println!("{result}");
        Ok::<(),Box<dyn std::error::Error>>(())
    })
}
