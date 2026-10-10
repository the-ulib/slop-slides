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
    worker: tokio::sync::Mutex<Option<(String, worker::Worker)>>,
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
                if let Some((_, mut old)) = worker.take() {
                    old.stop().await;
                }
            }
        });
    }
    async fn stop(&self) {
        self.0.epoch.fetch_add(1, Ordering::Relaxed);
        if let Some((_, mut old)) = self.0.worker.lock().await.take() {
            old.stop().await;
        }
    }
    async fn run(
        &self,
        request: SynthesisRequest,
        spool: &Path,
        profile: Option<PathBuf>,
        cancel: Cancellation,
        progress: Progress,
    ) -> Result<Artifact> {
        let kind = if profile.is_some() {
            models::Kind::Base
        } else {
            models::Kind::CustomVoice
        };
        let key = profile
            .as_ref()
            .map_or_else(|| "stock".into(), |p| p.to_string_lossy().into_owned());
        cancel.check()?;
        self.0.epoch.fetch_add(1, Ordering::Relaxed);
        let mut worker = self.0.worker.lock().await;
        let result: Result<Artifact> = async {
            cancel.check()?;
            if worker.as_ref().is_some_and(|(old, _)| old != &key) {
                if let Some((_, mut old)) = worker.take() {
                    old.stop().await;
                }
            }
            if worker.is_none() {
                progress("loading", 0, 0, "Verifying and loading local voice model…");
                let root = self.0.root.clone();
                let check = cancel.clone();
                tokio::task::spawn_blocking(move || kind.verify(&root, check.flag()))
                    .await
                    .map_err(|e| Error::msg(e.to_string()))??;
                *worker = Some((
                    key.clone(),
                    worker::Worker::start_voice(
                        self.0
                            .helper
                            .as_ref()
                            .ok_or_else(|| Error::msg("Local helper unavailable."))?,
                        &self.0.root,
                        kind,
                        profile.as_deref(),
                        &mut cancel.receiver(),
                    )
                    .await?,
                ));
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
                    .1
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
            if let Some((_, mut old)) = worker.take() {
                old.stop().await;
            }
        }
        drop(worker);
        self.idle();
        result
    }
    async fn install_kind(
        &self,
        kind: models::Kind,
        source: Option<PathBuf>,
        cancel: Cancellation,
        progress: Progress,
    ) -> Result<()> {
        if self.0.helper.is_none() {
            return Err(Error::msg("Local speech helper unavailable in this build."));
        }
        self.stop().await;
        let root = self.0.root.clone();
        tokio::task::spawn_blocking(move || {
            kind.install(&root, source.as_deref(), cancel.flag(), &|n, detail| {
                progress("installing", n, kind.total_bytes(), detail)
            })
        })
        .await
        .map_err(|e| Error::msg(e.to_string()))?
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
        Descriptor{id:QWEN_ID.into(),label:"Local Qwen".into(),contract_version:1,processing:"local".into(),engine_version:QWEN_ENGINE.into(),model_revision:models::pack().revision, ready:models::installed(&self.0.root),available:self.0.helper.is_some(),unavailable_reason:self.0.helper.is_none().then(||"The bundled local speech helper is unavailable in this build.".into()),voices: {
            let mut voices: Vec<Voice> = names.into_iter().map(|(id,name)|Voice{id:format!("preset:{id}"),name:name.into(), revision:None, ready:Some(models::installed(&self.0.root)), model_revision:None, engine_version:None, reference_language:None}).collect();
            for profile in self.profiles().unwrap_or_default() { voices.push(Voice { id:profile.id, name:profile.name, revision:Some(profile.revision), ready:Some(profile.ready && models::Kind::Base.installed(&self.0.root)), model_revision:Some(models::Kind::Base.pack().revision), engine_version:Some(crate::profiles::PROFILE_ENGINE.into()), reference_language:Some(profile.reference_language) }); } voices
        },languages:vec!["en".into(),"de".into()],pace:Pace{min:0.9,max:1.25,default:1.1,choices:vec![0.9,1.0,1.1,1.2]},supports_cloning:true,clone_ready:models::Kind::Base.installed(&self.0.root),clone_setup:Some(Setup{total_bytes:models::Kind::Base.total_bytes(),detail:"Personal voices use a separate local Base pack. Your recording and profile stay on this device.".into(),import_title:Some("Choose the pinned Qwen 0.6B Base pack".into())}),narration_controls:crate::narration::Controls::default(),narration_guidance:include_str!("../guidance/qwen-0.6b.md").into(),setup:Some(Setup{total_bytes:models::total_bytes(),detail:"Runs offline after setup with no service fees. Generation used about 3 GB of memory on the tested Mac.".into(),import_title:Some("Choose the pinned Qwen 0.6B CustomVoice pack".into())}),voice_hint:Some("Ryan and Aiden are English voices. Preview pronunciation in each intended language. Personal voices use the Base pack. Record in your primary language and preview every intended language; cross-language accents can remain.".into())}
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
            let profile = if request.voice_id.starts_with("profile:") {
                Some(
                    crate::profiles::Store::new(&self.0.root)
                        .resolve(&request.voice_id, request.voice_revision.as_deref())?,
                )
            } else {
                None
            };
            self.run(request, spool, profile, cancel, progress).await
        })
    }
    fn setup(
        &self,
        source: Option<PathBuf>,
        cancel: Cancellation,
        progress: Progress,
    ) -> Task<'_, ()> {
        Box::pin(self.install_kind(models::Kind::CustomVoice, source, cancel, progress))
    }
    fn setup_cloning(
        &self,
        source: Option<PathBuf>,
        cancel: Cancellation,
        progress: Progress,
    ) -> Task<'_, ()> {
        Box::pin(self.install_kind(models::Kind::Base, source, cancel, progress))
    }
    fn unload(&self) -> Task<'_, ()> {
        Box::pin(async move {
            self.stop().await;
            Ok(())
        })
    }
    fn profiles(&self) -> Result<Vec<crate::profiles::Profile>> {
        if self.0.root.as_os_str().is_empty() {
            return Ok(vec![]);
        }
        crate::profiles::Store::new(&self.0.root).list()
    }
    fn create_profile(
        &self,
        request: crate::profiles::Create,
        cancel: Cancellation,
        progress: Progress,
    ) -> Task<'_, crate::profiles::Profile> {
        Box::pin(async move {
            if !models::Kind::Base.installed(&self.0.root) {
                return Err(Error::msg("Set up the personal voice pack first."));
            }
            let store = crate::profiles::Store::new(&self.0.root);
            let (profile, path) = store.prepare(&request, &cancel)?;
            let result = async {
                self.stop().await;
                progress("loading", 0, 0, "Verifying the personal voice model…");
                let root = self.0.root.clone(); let check = cancel.clone();
                tokio::task::spawn_blocking(move || models::Kind::Base.verify(&root, check.flag())).await.map_err(|e| Error::msg(e.to_string()))??;
                let _lease = models::lock(&self.0.root, true)?;
                cancel.check()?;
                progress("profiling", 0, 0, "Creating your reusable voice profile…");
                let helper = self.0.helper.as_ref().ok_or_else(|| Error::msg("Local helper unavailable."))?;
                let mut command = tokio::process::Command::new(helper);
                command.arg("--create-profile").arg(models::Kind::Base.location(&self.0.root)).arg(path.join("reference.wav")).arg(path.join("transcript.txt")).arg(path.join("profile.bin")).kill_on_drop(true);
                let mut receiver = cancel.receiver();
                let output = tokio::select! {
                    _ = receiver.changed() => return Err(Error::msg("Presenter creation cancelled.")),
                    result = tokio::time::timeout(Duration::from_secs(180), command.output()) => result.map_err(|_| Error::msg("Presenter creation timed out."))??,
                };
                cancel.check()?;
                if !output.status.success() { return Err(Error::msg("Could not create this presenter. Try another clean recording and check its transcript.")); }
                store.finish(profile.clone())
            }.await;
            if result.is_err() {
                let _ = store.discard(&profile.revision);
            }
            result
        })
    }
    fn preview_profile<'a>(
        &'a self,
        token: &'a str,
        language: &'a str,
        cancel: Cancellation,
        progress: Progress,
    ) -> Task<'a, ()> {
        Box::pin(async move {
            if !["en", "de"].contains(&language) {
                return Err(Error::msg("Choose English or German."));
            }
            let store = crate::profiles::Store::new(&self.0.root);
            let profile = store.draft_profile(token)?;
            let path = store.data(token)?;
            let request = SynthesisRequest { text: if language == "de" { "Mit diesem Presenter kann ich weitere Präsentationen vertonen, ohne meine Stimme erneut aufzunehmen." } else { "This saved presenter lets me narrate another presentation without recording my voice again." }.into(), language: language.into(), voice_id: profile.id, voice_revision: Some(profile.revision), pace: 1.1 };
            let artifact = self
                .run(
                    request,
                    &path,
                    Some(path.join("profile.bin")),
                    cancel.clone(),
                    progress,
                )
                .await?;
            cancel.check()?;
            crate::profiles::atomic(
                &path.join(format!("preview-{language}.wav")),
                &fs::read(&artifact.path)?,
            )?;
            let _ = fs::remove_file(artifact.path);
            store.previewed(token, language)
        })
    }
    fn save_profile(
        &self,
        token: &str,
        replace: Option<&str>,
        name: Option<&str>,
    ) -> Result<crate::profiles::Profile> {
        crate::profiles::Store::new(&self.0.root).save(token, replace, name)
    }
    fn discard_profile(&self, token: &str) -> Result<()> {
        crate::profiles::Store::new(&self.0.root).discard(token)
    }
    fn rename_profile(&self, id: &str, name: &str) -> Result<()> {
        crate::profiles::Store::new(&self.0.root).rename(id, name)
    }
    fn delete_profile(&self, id: &str) -> Result<()> {
        crate::profiles::Store::new(&self.0.root).delete(id)
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
