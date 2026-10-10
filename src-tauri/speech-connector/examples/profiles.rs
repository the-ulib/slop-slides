//! Opt-in actual-model profile smoke; uses only the pinned public reference.
use speech_connector::{profiles, Cancellation, QwenConnector, SpeechProvider, SynthesisRequest};
use std::{path::PathBuf, sync::Arc};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 4, "profiles ROOT HELPER OUTPUT");
    let root = PathBuf::from(&args[1]);
    let helper = PathBuf::from(&args[2]);
    let out = PathBuf::from(&args[3]);
    assert!(!out.exists(), "output must be new");
    std::fs::create_dir_all(&out).unwrap();
    let reference = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/feasibility/listening-2026-10-09/l-german-public-reference.wav");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let connector = QwenConnector::new(root.clone(), Some(helper.clone()));
        let progress: speech_connector::Progress = Arc::new(|stage, _, _, detail| eprintln!("{stage}: {detail}"));
        let draft = connector.create_profile(profiles::Create { name:"Public German reference (test)".into(), language:"de".into(), transcript:"Eure Schoko-Bonbons sind sagenhaft lecker! Europa und Asien zusammengenommen wird auch als Eurasien bezeichnet. Euer Plan hat ja toll geklappt.".into(), reference, authorized:true }, Cancellation::default(), progress.clone()).await.unwrap();
        for language in ["de", "en"] {
            connector.preview_profile(&draft.revision, language, Cancellation::default(), progress.clone()).await.unwrap();
            let preview = profiles::Store::new(&root).preview_file(&draft.revision, language).unwrap();
            std::fs::copy(preview, out.join(format!("preview-{language}.wav"))).unwrap();
        }
        let profile = connector.save_profile(&draft.revision, None, None).unwrap();
        drop(connector);
        // A fresh connector reloads the persisted profile; reference is not supplied.
        let fresh = QwenConnector::new(root.clone(), Some(helper));
        let description = fresh.describe();
        assert!(description.voices.iter().any(|v| v.id == profile.id && v.revision.as_deref() == Some(&profile.revision)));
        let request = SynthesisRequest { text:"Eine gespeicherte Stimme bleibt nach dem Neustart verfügbar. So kann ich mehrere Präsentationen mit demselben Presenter vertonen.".into(), language:"de".into(), voice_id:profile.id.clone(), pace:1.1, voice_revision:Some(profile.revision.clone()) };
        let artifact = fresh.synthesize(request.clone(), &out, Cancellation::default(), progress.clone()).await.unwrap();
        std::fs::rename(artifact.path, out.join("reused-de.wav")).unwrap();
        fresh.rename_profile(&profile.id, "Renamed public test").unwrap();
        assert_eq!(fresh.profiles().unwrap()[0].revision, profile.revision);
        let cancelled = Cancellation::default(); cancelled.cancel();
        assert!(fresh.synthesize(request.clone(), &out, cancelled, progress.clone()).await.is_err());
        let artifact = fresh.synthesize(request, &out, Cancellation::default(), progress).await.unwrap();
        assert_eq!(std::fs::read(out.join("reused-de.wav")).unwrap(), std::fs::read(&artifact.path).unwrap());
        std::fs::remove_file(artifact.path).unwrap();
        std::fs::write(out.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({"profile":profile,"freshConnectorReusedProfile":true,"renameKeptRevision":true,"cancelRestartSameBytes":true})).unwrap()).unwrap();
        println!("Profile smoke passed: {}",out.display());
    });
}
