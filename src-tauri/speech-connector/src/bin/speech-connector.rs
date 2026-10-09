//! Standalone deterministic consumer. JSON on stdout; progress on stderr.
use speech_connector::{
    Cancellation, FixtureProvider, QwenConnector, SpeechProvider, SynthesisRequest,
};
use std::{path::PathBuf, sync::Arc};
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 && args.len() != 5 {
        return Err("Usage: speech-connector describe ROOT HELPER | synthesize ROOT HELPER REQUEST.json NEW_SPOOL | fixture REQUEST.json NEW_SPOOL".into());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let provider:Box<dyn SpeechProvider>=if args[0]=="fixture" { Box::new(FixtureProvider) } else { Box::new(QwenConnector::new(PathBuf::from(&args[1]),Some(PathBuf::from(&args[2])))) };
        if args[0]=="describe" { println!("{}",serde_json::to_string(&provider.describe())?); return Ok(()); }
        let (request_path,spool)=match args[0].as_str(){ "synthesize" if args.len()==5 => (&args[3],PathBuf::from(&args[4])), "fixture" if args.len()==3 => (&args[1],PathBuf::from(&args[2])), _=>return Err("Invalid command/arguments.".into()) };
        // Never overwrite an existing directory or arbitrary caller file.
        std::fs::create_dir(&spool)?;
        let bytes=std::fs::read(request_path)?;
        let requests:Vec<SynthesisRequest>=if bytes.starts_with(b"["){serde_json::from_slice(&bytes)?}else{vec![serde_json::from_slice(&bytes)?]};
        let mut outputs=Vec::new();
        for request in requests {
            let artifact=provider.synthesize(request,&spool,Cancellation::default(),Arc::new(|_,_,_,detail|eprintln!("{detail}"))).await?;
            outputs.push(serde_json::json!({"path":artifact.path,"sampleRate":artifact.sample_rate,"samples":artifact.samples}));
        }
        println!("{}",serde_json::to_string(&outputs)?); Ok(())
    })
}
