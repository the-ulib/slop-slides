use crate::{
    audio, models, segments, worker, Artifact, Cancellation, Descriptor, Error, Pace, Progress,
    Result, Setup, SpeechProvider, SynthesisRequest, Task, Voice, QWEN_ENGINE, QWEN_ID,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
struct Inner {
    root: PathBuf,
    helper: Option<PathBuf>,
    worker: tokio::sync::Mutex<Option<worker::Worker>>,
    epoch: AtomicU64,
}
#[derive(Clone)]
pub struct QwenConnector(Arc<Inner>);
impl QwenConnector {
    pub fn new(root: PathBuf, helper: Option<PathBuf>) -> Self {
        Self(Arc::new(Inner {
            root,
            helper,
            worker: tokio::sync::Mutex::new(None),
            epoch: AtomicU64::new(0),
        }))
    }
    fn idle(&self) {
        let connector = self.clone();
        let epoch = self.0.epoch.load(Ordering::Relaxed);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(120)).await;
            let mut worker = connector.0.worker.lock().await;
            if connector.0.epoch.load(Ordering::Relaxed) == epoch {
                if let Some(mut old) = worker.take() {
                    old.stop().await;
                }
            }
        });
    }
    async fn stop(&self) {
        self.0.epoch.fetch_add(1, Ordering::Relaxed);
        if let Some(mut old) = self.0.worker.lock().await.take() {
            old.stop().await;
        }
    }
}
impl SpeechProvider for QwenConnector {
    fn describe(&self) -> Descriptor {
        let names = [
            ("ryan", "Ryan"),
            ("aiden", "Aiden"),
            ("vivian", "Vivian"),
            ("serena", "Serena"),
            ("uncle_fu", "Uncle Fu"),
            ("dylan", "Dylan"),
            ("eric", "Eric"),
            ("ono_anna", "Ono Anna"),
            ("sohee", "Sohee"),
        ];
        Descriptor{id:QWEN_ID.into(),label:"Local Qwen".into(),contract_version:1,processing:"local".into(),engine_version:QWEN_ENGINE.into(),model_revision:models::pack().revision, ready:models::installed(&self.0.root),available:self.0.helper.is_some(),unavailable_reason:self.0.helper.is_none().then(||"The bundled local speech helper is unavailable in this build.".into()),voices:names.into_iter().map(|(id,name)|Voice{id:format!("preset:{id}"),name:name.into()}).collect(),languages:vec!["en".into(),"de".into()],pace:Pace{min:0.9,max:1.25,default:1.1,choices:vec![0.9,1.0,1.1,1.2]},supports_cloning:false,narration_controls:crate::narration::Controls::default(),narration_guidance:include_str!("../guidance/qwen-0.6b.md").into(),setup:Some(Setup{total_bytes:models::total_bytes(),detail:"Runs offline after setup with no service fees. Generation used about 3 GB of memory on the tested Mac.".into(),import_title:Some("Choose the pinned Qwen 0.6B CustomVoice pack".into())}),voice_hint:Some("Ryan and Aiden are English voices. Preview pronunciation in each intended language. Saved personal voices come later.".into())}
    }
    fn synthesize<'a>(
        &'a self,
        request: SynthesisRequest,
        spool: &'a Path,
        cancel: Cancellation,
        progress: Progress,
    ) -> Task<'a, Artifact> {
        Box::pin(async move {
            self.describe().validate_plain(&request)?;
            cancel.check()?;
            self.0.epoch.fetch_add(1, Ordering::Relaxed);
            let mut worker = self.0.worker.lock().await;
            let result: Result<Artifact> = async {
                cancel.check()?;
                if worker.is_none() {
                    progress("loading", 0, 0, "Verifying and loading local voice model…");
                    let root = self.0.root.clone();
                    let check = cancel.clone();
                    tokio::task::spawn_blocking(move || models::verify(&root, check.flag()))
                        .await
                        .map_err(|e| Error::msg(e.to_string()))??;
                    *worker = Some(
                        worker::Worker::start(
                            self.0
                                .helper
                                .as_ref()
                                .ok_or_else(|| Error::msg("Local helper unavailable."))?,
                            &self.0.root,
                            &mut cancel.receiver(),
                        )
                        .await?,
                    );
                }
                fs::create_dir_all(spool)?;
                let chunks = segments::segments(&request.text);
                let mut pcm = Vec::new();
                let mut receiver = cancel.receiver();
                for (part, text) in chunks.iter().enumerate() {
                    let output = spool.join(format!("{}.wav", uuid::Uuid::new_v4()));
                    let detail = format!("Passage {} of {}", part + 1, chunks.len());
                    progress("generating", part as u64, chunks.len() as u64, &detail);
                    let n = worker
                        .as_mut()
                        .unwrap()
                        .generate(&request, text, &output, &mut receiver, &|frames| {
                            progress(
                                "generating",
                                part as u64,
                                chunks.len() as u64,
                                &format!("{detail} · {:.0}s synthesized", frames as f64 / 12.5),
                            )
                        })
                        .await?;
                    let decoded = audio::decode(&fs::read(&output)?)?;
                    if decoded.sample_rate != 24000 || decoded.samples.len() != n {
                        return Err(Error::msg(
                            "Worker audio length/format does not match its result.",
                        ));
                    }
                    if !pcm.is_empty() {
                        pcm.resize(pcm.len() + 2880, 0);
                    }
                    pcm.extend(decoded.samples);
                    let _ = fs::remove_file(output);
                    if pcm.len() > 600 * 24000 {
                        return Err(Error::msg(
                            "Recording exceeds ten minutes. Shorten the script.",
                        ));
                    }
                }
                cancel.check()?;
                let path = spool.join(format!("{}.wav", uuid::Uuid::new_v4()));
                fs::write(&path, audio::encode(&pcm, 24000))?;
                Ok(Artifact {
                    path,
                    sample_rate: 24000,
                    samples: pcm.len(),
                })
            }
            .await;
            if result.is_err() {
                if let Some(mut old) = worker.take() {
                    old.stop().await;
                }
            }
            drop(worker);
            self.idle();
            result
        })
    }
    fn setup(
        &self,
        source: Option<PathBuf>,
        cancel: Cancellation,
        progress: Progress,
    ) -> Task<'_, ()> {
        Box::pin(async move {
            if !self.describe().available {
                return Err(Error::msg("Local speech helper unavailable in this build."));
            }
            self.stop().await;
            let root = self.0.root.clone();
            tokio::task::spawn_blocking(move || {
                models::install(&root, source.as_deref(), cancel.flag(), &|n, detail| {
                    progress("installing", n, models::total_bytes(), detail)
                })
            })
            .await
            .map_err(|e| Error::msg(e.to_string()))?
        })
    }
    fn remove(&self) -> Task<'_, ()> {
        Box::pin(async move {
            self.stop().await;
            let root = self.0.root.clone();
            tokio::task::spawn_blocking(move || models::remove(&root))
                .await
                .map_err(|e| Error::msg(e.to_string()))?
        })
    }
}
