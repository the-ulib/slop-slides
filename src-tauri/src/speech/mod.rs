//! Local stock narration: one active job, a warm native worker, immutable WAV takes.
pub mod cache;
mod models;
mod worker;
use crate::{
    deck,
    error::{Error, Result},
    narration,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::watch;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub deck_id: Option<String>,
    pub source_revision: Option<u64>,
    pub stage: String,
    pub completed: u64,
    pub total: u64,
    pub detail: String,
}
struct Active {
    job: Job,
    cancel: Arc<AtomicBool>,
    signal: watch::Sender<bool>,
}
#[derive(Default)]
struct Inner {
    active: Mutex<Option<Active>>,
    worker: tokio::sync::Mutex<Option<worker::Worker>>,
    epoch: AtomicU64,
}
#[derive(Clone, Default)]
pub struct SpeechManager(Arc<Inner>);
struct Lease {
    manager: SpeechManager,
    id: String,
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut active = self.manager.0.active.lock().expect("speech job");
        if active.as_ref().is_some_and(|a| a.job.id == self.id) {
            *active = None;
        }
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    installed: bool,
    runtime_available: bool,
    total_bytes: u64,
    engine_version: &'static str,
    job: Option<Job>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Event {
    job: Job,
    error: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationResult {
    pub generated: usize,
    pub reused: usize,
    pub superseded: usize,
}
fn root(app: &AppHandle) -> Result<PathBuf> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| Error::msg(e.to_string()))?
        .join("speech"))
}
fn runtime(app: &AppHandle) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let bundled = app
        .path()
        .resource_dir()
        .ok()?
        .join("speech-runtime/slopslide-speech");
    if bundled.is_file() {
        return Some(bundled);
    }
    if cfg!(debug_assertions) {
        let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("speech-runtime/slopslide-speech");
        if dev.is_file() {
            return Some(dev);
        }
    }
    None
}
impl SpeechManager {
    pub fn status(&self, app: &AppHandle) -> Result<Status> {
        Ok(Status {
            installed: models::installed(&root(app)?),
            runtime_available: runtime(app).is_some(),
            total_bytes: models::total_bytes(),
            engine_version: cache::ENGINE,
            job: self
                .0
                .active
                .lock()
                .expect("speech job")
                .as_ref()
                .map(|a| a.job.clone()),
        })
    }
    fn begin(
        &self,
        id: String,
        kind: &str,
        deck_id: Option<String>,
        revision: Option<u64>,
        total: u64,
    ) -> Result<(Lease, Arc<AtomicBool>, watch::Receiver<bool>)> {
        uuid::Uuid::parse_str(&id).map_err(|_| Error::msg("Invalid speech job ID."))?;
        let mut active = self.0.active.lock().expect("speech job");
        if active.is_some() {
            return Err(Error::msg(
                "Another speech job is running. Wait or cancel it first.",
            ));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (signal, receiver) = watch::channel(false);
        *active = Some(Active {
            job: Job {
                id: id.clone(),
                kind: kind.into(),
                deck_id,
                source_revision: revision,
                stage: "starting".into(),
                completed: 0,
                total,
                detail: "Starting local speech…".into(),
            },
            cancel: cancel.clone(),
            signal,
        });
        self.0.epoch.fetch_add(1, Ordering::Relaxed);
        Ok((
            Lease {
                manager: self.clone(),
                id,
            },
            cancel,
            receiver,
        ))
    }
    #[allow(clippy::too_many_arguments)] // A single event carries stage, counts, detail and failure.
    fn report(
        &self,
        app: &AppHandle,
        id: &str,
        stage: &str,
        completed: u64,
        total: u64,
        detail: &str,
        error: Option<String>,
    ) {
        let mut active = self.0.active.lock().expect("speech job");
        if let Some(a) = active.as_mut().filter(|a| a.job.id == id) {
            a.job.stage = stage.into();
            a.job.completed = completed;
            a.job.total = total;
            a.job.detail = detail.into();
            let _ = app.emit(
                "speech-event",
                Event {
                    job: a.job.clone(),
                    error,
                },
            );
        }
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        let active = self.0.active.lock().expect("speech job");
        if let Some(a) = active.as_ref().filter(|a| a.job.id == id) {
            a.cancel.store(true, Ordering::Relaxed);
            let _ = a.signal.send(true);
        }
        Ok(())
    }
    fn idle(&self) {
        let manager = self.clone();
        let epoch = self.0.epoch.load(Ordering::Relaxed);
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(120)).await;
            let mut worker = manager.0.worker.lock().await;
            if manager.0.epoch.load(Ordering::Relaxed) == epoch
                && manager.0.active.lock().expect("speech job").is_none()
            {
                if let Some(mut old) = worker.take() {
                    old.stop().await;
                }
            }
        });
    }
    pub async fn install(&self, app: AppHandle, id: String, source: Option<String>) -> Result<()> {
        if runtime(&app).is_none() {
            return Err(Error::msg(
                "Local speech requires the macOS speech helper in this build.",
            ));
        }
        let (lease, cancel, _) =
            self.begin(id.clone(), "setup", None, None, models::total_bytes())?;
        if let Some(mut old) = self.0.worker.lock().await.take() {
            old.stop().await;
        }
        let root = root(&app)?;
        let manager = self.clone();
        let progress_app = app.clone();
        let progress_id = id.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let mut last = Instant::now() - Duration::from_secs(1);
            let last = Mutex::new(&mut last);
            models::install(
                &root,
                source.as_deref().map(Path::new),
                &cancel,
                &|bytes, detail| {
                    let mut previous = last.lock().expect("progress throttle");
                    if previous.elapsed() >= Duration::from_millis(150)
                        || bytes == models::total_bytes()
                    {
                        **previous = Instant::now();
                        manager.report(
                            &progress_app,
                            &progress_id,
                            "installing",
                            bytes,
                            models::total_bytes(),
                            detail,
                            None,
                        );
                    }
                },
            )
        })
        .await
        .map_err(|e| Error::msg(e.to_string()))?;
        self.report(
            &app,
            &id,
            if result.is_ok() { "complete" } else { "failed" },
            0,
            0,
            if result.is_ok() {
                "Local speech is ready."
            } else {
                "Setup stopped; retry to resume."
            },
            result.as_ref().err().map(ToString::to_string),
        );
        drop(lease);
        result
    }
    pub async fn remove(&self, app: &AppHandle) -> Result<()> {
        let (lease, _, _) =
            self.begin(uuid::Uuid::new_v4().to_string(), "remove", None, None, 0)?;
        if let Some(mut old) = self.0.worker.lock().await.take() {
            old.stop().await;
        }
        let root = root(app)?;
        let result = tauri::async_runtime::spawn_blocking(move || models::remove(&root))
            .await
            .map_err(|e| Error::msg(e.to_string()))?;
        drop(lease);
        result
    }
    pub async fn generate(
        &self,
        app: AppHandle,
        id: String,
        deck_id: String,
        slide: Option<String>,
    ) -> Result<GenerationResult> {
        let dir = deck::deck_dir(&app, &deck_id)?;
        let doc = narration::load(&dir)?;
        let deck = deck::load(&dir, &deck_id)?;
        let mut sources = Vec::new();
        for s in &deck.slides {
            if slide.as_ref().map_or(!s.hidden, |id| id == &s.id)
                && doc
                    .manifest
                    .slides
                    .get(&s.id)
                    .is_some_and(|n| !n.text.trim().is_empty())
            {
                sources.push((
                    s.id.clone(),
                    cache::Source::from_manifest(&doc.manifest, &s.id)?,
                ));
            }
        }
        if sources.is_empty() {
            return Err(Error::msg("Write a script for a visible slide first."));
        }
        let root = root(&app)?;
        let bin = runtime(&app)
            .ok_or_else(|| Error::msg("Local speech is not included in this build."))?;
        if !models::installed(&root) {
            return Err(Error::msg("Set up the local voice pack first."));
        }
        let total = sources.len() as u64;
        let (lease, cancel, mut receiver) = self.begin(
            id.clone(),
            "generation",
            Some(deck_id),
            Some(doc.manifest.revision),
            total,
        )?;
        let mut worker = self.0.worker.lock().await;
        let result = async {
            let jobs = cache::safe_directory(&dir, "jobs")?.join(&id);
            fs::create_dir_all(&jobs)?;
            let _temp = Temp(jobs.clone());
            let mut result = GenerationResult {
                generated: 0,
                reused: 0,
                superseded: 0,
            };
            for (index, (slide, source)) in sources.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::msg(
                        "Audio generation cancelled. Previous recordings are kept.",
                    ));
                }
                let take = if let Some(take) = cache::find(&dir, &source.key())? {
                    result.reused += 1;
                    take
                } else {
                    if worker.is_none() {
                        self.report(
                            &app,
                            &id,
                            "loading",
                            index as u64,
                            total,
                            "Loading local voice model…",
                            None,
                        );
                        let model_root = root.clone();
                        let check = cancel.clone();
                        tauri::async_runtime::spawn_blocking(move || {
                            models::verify(&model_root, &check)
                        })
                        .await
                        .map_err(|e| Error::msg(e.to_string()))??;
                        *worker = Some(worker::Worker::start(&bin, &root, &mut receiver).await?);
                    }
                    let chunks = cache::segments(&source.text);
                    let mut pcm = Vec::new();
                    for (part, text) in chunks.iter().enumerate() {
                        let output = jobs.join("segment.wav");
                        let detail = format!(
                            "Slide {} of {} · passage {} of {}",
                            index + 1,
                            total,
                            part + 1,
                            chunks.len()
                        );
                        self.report(&app, &id, "generating", index as u64, total, &detail, None);
                        let n = worker
                            .as_mut()
                            .expect("worker loaded")
                            .generate(source, text, &output, &mut receiver, &|frames| {
                                self.report(
                                    &app,
                                    &id,
                                    "generating",
                                    index as u64,
                                    total,
                                    &format!("{detail} · {:.0}s synthesized", frames as f64 / 12.5),
                                    None,
                                )
                            })
                            .await?;
                        let mut segment = cache::decode_wav(&fs::read(&output)?)?;
                        if segment.len() != n {
                            return Err(Error::msg(
                                "Worker audio length does not match its result.",
                            ));
                        }
                        if !pcm.is_empty() {
                            pcm.resize(pcm.len() + 2880, 0);
                        }
                        pcm.append(&mut segment);
                        if pcm.len() > 600 * 24000 {
                            return Err(Error::msg(
                                "Recording exceeds ten minutes per slide. Shorten the script.",
                            ));
                        }
                    }
                    if cancel.load(Ordering::Relaxed) {
                        return Err(Error::msg("Audio generation cancelled."));
                    }
                    result.generated += 1;
                    cache::publish(&dir, source.clone(), &pcm)?
                };
                let active = self.0.active.lock().expect("speech job");
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::msg("Audio generation cancelled."));
                }
                if !cache::accept(&dir, slide, &take)? {
                    result.superseded += 1;
                }
                drop(active);
                self.report(
                    &app,
                    &id,
                    "generating",
                    index as u64 + 1,
                    total,
                    "Recording saved locally.",
                    None,
                );
            }
            Ok(result)
        }
        .await;
        if result.is_err() {
            if let Some(mut old) = worker.take() {
                old.stop().await;
            }
        }
        drop(worker);
        self.report(
            &app,
            &id,
            if result.is_ok() { "complete" } else { "failed" },
            total,
            total,
            if result.is_ok() {
                "Audio ready."
            } else {
                "Generation stopped; previous recordings are kept."
            },
            result.as_ref().err().map(ToString::to_string),
        );
        drop(lease);
        self.idle();
        result
    }
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub fn takes(app: &AppHandle, id: &str) -> Result<BTreeMap<String, cache::Take>> {
    let dir = deck::deck_dir(app, id)?;
    let doc = narration::load(&dir)?;
    let mut takes = BTreeMap::new();
    for (slide, script) in doc.manifest.slides {
        if let Some(id) = script.accepted_take_id {
            if let Some(take) = cache::read(&dir, &id)? {
                takes.insert(slide, take);
            }
        }
    }
    Ok(takes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_job_runs_and_stale_cancellation_cannot_stop_the_next() {
        let manager = SpeechManager::default();
        let id = uuid::Uuid::new_v4().to_string();
        let (lease, cancel, signal) = manager
            .begin(id.clone(), "generation", Some("deck".into()), Some(1), 1)
            .unwrap();
        assert!(manager
            .begin(uuid::Uuid::new_v4().to_string(), "setup", None, None, 1)
            .is_err());
        manager.cancel("stale").unwrap();
        assert!(!cancel.load(Ordering::Relaxed));
        manager.cancel(&id).unwrap();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(*signal.borrow());
        drop(lease);
        let (_next, next_cancel, _) = manager
            .begin(
                uuid::Uuid::new_v4().to_string(),
                "generation",
                None,
                None,
                1,
            )
            .unwrap();
        manager.cancel(&id).unwrap();
        assert!(!next_cancel.load(Ordering::Relaxed));
    }
}
