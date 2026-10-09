//! Frozen narrated-video jobs. Timeline and accepted audio are provider independent.
use crate::{
    deck,
    error::{Error, Result},
    html, narration,
    speech::cache,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

pub const RATE: u64 = 24_000;
pub const FPS: u64 = 30;
const FRAME_SAMPLES: u64 = RATE / FPS;
const MAX_SAMPLES: u64 = RATE * 60 * 180;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: String,
    pub start_sample: u64,
    pub end_sample: u64,
    pub start_frame: u64,
    pub end_frame: u64,
    pub audio_start_sample: u64,
    pub take_id: Option<String>,
    pub take_sha256: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub id: String,
    pub deck_id: String,
    pub source_revision: u64,
    pub sample_rate: u64,
    pub fps: u64,
    pub total_samples: u64,
    pub total_frames: u64,
    pub slides: Vec<Segment>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: String,
    pub stage: String,
    pub completed: u64,
    pub total: u64,
}
#[derive(Clone)]
struct Job {
    root: PathBuf,
    cancelled: Arc<AtomicBool>,
    timeline: Option<Timeline>,
}
#[derive(Default)]
pub struct VideoManager {
    jobs: Mutex<HashMap<String, Job>>,
    active: Mutex<Option<String>>,
}
fn cancelled(flag: &AtomicBool) -> Result<()> {
    if flag.load(Ordering::Relaxed) {
        Err(Error::msg("Video job cancelled."))
    } else {
        Ok(())
    }
}

/// Cumulative nearest-frame boundaries keep rounding error below half a frame;
/// the final frame is padded up so no audio is clipped.
pub fn timeline(
    source: &str,
    manifest: &narration::Manifest,
    dir: &Path,
    job: &str,
    deck_id: &str,
) -> Result<Timeline> {
    let spans = html::find_slides(source);
    let mut samples = 0;
    let mut slides = Vec::new();
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    for (index, span) in spans.iter().enumerate().filter(|(_, s)| s.hidden.is_none()) {
        let label = format!("Slide {}", index + 1);
        let Some(id) = &span.id else {
            issues.push(format!("{label}: a stable slide ID is required."));
            continue;
        };
        let Some(script) = manifest.slides.get(id) else {
            issues.push(format!(
                "{label}: add speech or an explicit silent duration in Narration."
            ));
            continue;
        };
        let (duration, lead, take_id, take_sha256) = if script.text.trim().is_empty() {
            match script.silent_duration_ms {
                Some(ms) if ms > 0 => (u64::from(ms) * RATE / 1000, 0, None, None),
                _ => {
                    issues.push(format!("{label}: choose a silent duration in Narration."));
                    continue;
                }
            }
        } else {
            let take = script
                .accepted_take_id
                .as_deref()
                .map(|id| cache::read(dir, id))
                .transpose()?
                .flatten();
            let Some(take) = take else {
                issues.push(format!("{label}: generate narration audio first."));
                continue;
            };
            let expected = cache::Source::from_manifest(manifest, id)?;
            if take.source != expected {
                issues.push(if take.source.text != expected.text {
                    format!("{label}: the script changed; regenerate its audio.")
                } else {
                    format!("{label}: speech settings changed. Restore recording settings in Narration or regenerate its audio.")
                });
                continue;
            }
            let lead = u64::from(script.lead_in_ms) * RATE / 1000;
            if script.reviewed_slide_hash.as_deref()
                != Some(&format!(
                    "{}:{}",
                    html::shell_hash(source, &spans, &html::find_sections(source)),
                    html::content_hash(&source[span.range.clone()])
                ))
            {
                warnings.push(format!(
                    "{label}: review the narration against the current slide."
                ));
            }
            (
                lead + take.samples as u64 + u64::from(script.tail_ms) * RATE / 1000,
                lead,
                Some(take.id),
                Some(take.sha256),
            )
        };
        let end = samples + duration;
        if end > MAX_SAMPLES {
            return Err(Error::msg("Narrated videos are limited to three hours."));
        }
        slides.push(Segment {
            id: id.clone(),
            start_sample: samples,
            end_sample: end,
            start_frame: (samples + FRAME_SAMPLES / 2) / FRAME_SAMPLES,
            end_frame: (end + FRAME_SAMPLES / 2) / FRAME_SAMPLES,
            audio_start_sample: samples + lead,
            take_id,
            take_sha256,
        });
        samples = end;
    }
    if !issues.is_empty() {
        return Err(Error::msg(issues.join("\n")));
    }
    if slides.is_empty() {
        return Err(Error::msg("The deck has no visible slides."));
    }
    let lower = source.to_ascii_lowercase();
    if ["<video", "<canvas", "<iframe", "<animate", "<audio"]
        .iter()
        .any(|s| lower.contains(s))
    {
        warnings.push("Embedded media, canvas and animations are flattened to a still image; embedded sound is excluded.".into());
    }
    warnings.push(
        "Static slides at 1080p / 30 fps. Transitions and slide animations are flattened.".into(),
    );
    let frames = samples.div_ceil(FRAME_SAMPLES);
    slides.last_mut().expect("nonempty").end_frame = frames;
    Ok(Timeline {
        id: job.into(),
        deck_id: deck_id.into(),
        source_revision: manifest.revision,
        sample_rate: RATE,
        fps: FPS,
        total_samples: frames * FRAME_SAMPLES,
        total_frames: frames,
        slides,
        warnings,
    })
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn copy_assets(
    src: &Path,
    dst: &Path,
    originals: &mut Vec<(PathBuf, String)>,
    bytes: &mut u64,
    flag: &AtomicBool,
) -> Result<()> {
    cancelled(flag)?;
    if !src.exists() {
        return Ok(());
    }
    if fs::symlink_metadata(src)?.file_type().is_symlink() {
        return Err(Error::msg("Video assets cannot use symbolic links."));
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        cancelled(flag)?;
        let entry = entry?;
        let path = entry.path();
        let info = fs::symlink_metadata(&path)?;
        if info.file_type().is_symlink() {
            return Err(Error::msg("Video assets cannot use symbolic links."));
        }
        if info.is_dir() {
            copy_assets(&path, &dst.join(entry.file_name()), originals, bytes, flag)?;
        } else if info.is_file() {
            *bytes += info.len();
            if info.len() > 100_000_000 || *bytes > 1_000_000_000 || originals.len() >= 10_000 {
                return Err(Error::msg(
                    "Video assets exceed the export limit (100 MB per file, 1 GB total).",
                ));
            }
            let data = fs::read(&path)?;
            originals.push((path, hash(&data)));
            fs::write(dst.join(entry.file_name()), data)?;
        } else {
            return Err(Error::msg("Video assets must be regular files."));
        }
    }
    Ok(())
}
fn silence(file: &mut impl Write, samples: u64, flag: &AtomicBool) -> Result<()> {
    let zeros = [0u8; 48_000];
    let mut bytes = samples * 2;
    while bytes > 0 {
        cancelled(flag)?;
        let size = bytes.min(zeros.len() as u64) as usize;
        file.write_all(&zeros[..size])?;
        bytes -= size as u64;
    }
    Ok(())
}
fn write_audio(dir: &Path, root: &Path, timeline: &Timeline, flag: &AtomicBool) -> Result<()> {
    let mut file = fs::File::create(root.join("timeline.wav"))?;
    stream_audio(dir, timeline, flag, &mut file)?;
    file.sync_all()?;
    Ok(())
}
fn stream_audio(
    dir: &Path,
    timeline: &Timeline,
    flag: &AtomicBool,
    file: &mut impl Write,
) -> Result<()> {
    let length = u32::try_from(timeline.total_samples * 2)
        .map_err(|_| Error::msg("Audio timeline exceeds WAV limits."))?;
    file.write_all(b"RIFF")?;
    file.write_all(&(length + 36).to_le_bytes())?;
    file.write_all(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00")?;
    file.write_all(&(RATE as u32).to_le_bytes())?;
    file.write_all(&((RATE * 2) as u32).to_le_bytes())?;
    file.write_all(b"\x02\x00\x10\x00data")?;
    file.write_all(&length.to_le_bytes())?;
    let mut cursor = 0;
    for segment in &timeline.slides {
        silence(file, segment.audio_start_sample - cursor, flag)?;
        cursor = segment.audio_start_sample;
        if let Some(id) = &segment.take_id {
            let take = cache::read(dir, id)?
                .ok_or_else(|| Error::msg("Accepted audio disappeared while freezing the job."))?;
            let bytes = fs::read(cache::take_path(dir, id, "wav")?)?;
            if segment.take_sha256.as_deref() != Some(hash(&bytes).as_str()) {
                return Err(Error::msg("Accepted audio changed while freezing the job."));
            }
            let pcm = cache::decode_wav(&bytes)?;
            if segment.take_sha256.as_deref() != Some(take.sha256.as_str())
                || cursor + pcm.len() as u64 > segment.end_sample
                || pcm.len() != take.samples
            {
                return Err(Error::msg("Accepted audio changed while freezing the job."));
            }
            for chunk in pcm.chunks(24_000) {
                cancelled(flag)?;
                let bytes: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
                file.write_all(&bytes)?;
            }
            cursor += pcm.len() as u64;
        }
        silence(file, segment.end_sample - cursor, flag)?;
        cursor = segment.end_sample;
    }
    silence(file, timeline.total_samples - cursor, flag)?;
    Ok(())
}

pub fn freeze(dir: &Path, root: &Path, job: &str, id: &str, flag: &AtomicBool) -> Result<Timeline> {
    let source = fs::read_to_string(dir.join(deck::DECK_FILE))?;
    if source.len() > 10_000_000 {
        return Err(Error::msg("Deck HTML exceeds the 10 MB export limit."));
    }
    let document = narration::load(dir)?;
    let timeline = timeline(&source, &document.manifest, dir, job, id)?;
    let mut originals = Vec::new();
    copy_assets(
        &dir.join("assets"),
        &root.join("assets"),
        &mut originals,
        &mut 0,
        flag,
    )?;
    fs::write(root.join("deck.html"), &source)?;
    write_audio(dir, root, &timeline, flag)?;
    // Reject a mixed snapshot. Later edits are allowed after these frozen bytes exist.
    if fs::read_to_string(dir.join(deck::DECK_FILE))? != source
        || narration::load(dir)?.version != document.version
        || originals
            .iter()
            .any(|(path, digest)| fs::read(path).map(|b| hash(&b) != *digest).unwrap_or(true))
    {
        return Err(Error::msg(
            "The deck changed while preparing video. Retry to use its latest version.",
        ));
    }
    fs::write(
        root.join("job.json"),
        serde_json::to_vec(&timeline).map_err(|e| Error::msg(e.to_string()))?,
    )?;
    cancelled(flag)?;
    Ok(timeline)
}

struct Active<'a>(&'a VideoManager, String);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        let mut active = self.0.active.lock().expect("video mutex");
        if active.as_ref() == Some(&self.1) {
            *active = None;
        }
    }
}
impl VideoManager {
    fn acquire(&self, id: &str) -> Result<Active<'_>> {
        let mut active = self.active.lock().expect("video mutex");
        if active.is_some() {
            return Err(Error::msg("Another video job is running."));
        }
        *active = Some(id.into());
        Ok(Active(self, id.into()))
    }
    pub fn cancel(&self, id: &str) -> Result<()> {
        if let Some(job) = self.jobs.lock().expect("video mutex").get(id) {
            job.cancelled.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
    pub fn release(&self, id: &str) -> Result<()> {
        if self.active.lock().expect("video mutex").as_deref() == Some(id) {
            return self.cancel(id);
        }
        if let Some(job) = self.jobs.lock().expect("video mutex").remove(id) {
            fs::remove_dir_all(job.root)?;
        }
        Ok(())
    }
    pub fn file(&self, id: &str, name: &str) -> Option<PathBuf> {
        let jobs = self.jobs.lock().ok()?;
        let job = jobs.get(id)?;
        let timeline = job.timeline.as_ref()?;
        let allowed = name == "timeline.wav"
            || name
                .strip_prefix("frame-")
                .and_then(|s| s.strip_suffix(".png"))
                .and_then(|n| n.parse::<usize>().ok())
                .is_some_and(|index| {
                    index < timeline.slides.len() && name == format!("frame-{index}.png")
                });
        allowed.then(|| job.root.join(name))
    }
    pub async fn prepare(&self, app: &AppHandle, id: &str, job_id: &str) -> Result<Timeline> {
        if !cfg!(target_os = "macos") {
            return Err(Error::msg(
                "Narrated video preview/export currently requires macOS.",
            ));
        }
        uuid::Uuid::parse_str(job_id).map_err(|_| Error::msg("Invalid video job ID."))?;
        let _active = self.acquire(job_id)?;
        let root = app
            .path()
            .app_cache_dir()
            .map_err(|e| Error::msg(e.to_string()))?
            .join("video")
            .join(job_id);
        let cancelled = Arc::new(AtomicBool::new(false));
        if self.jobs.lock().expect("video mutex").contains_key(job_id) {
            return Err(Error::msg("Video job ID already exists."));
        }
        fs::create_dir_all(&root)?;
        self.jobs.lock().expect("video mutex").insert(
            job_id.into(),
            Job {
                root: root.clone(),
                cancelled: cancelled.clone(),
                timeline: None,
            },
        );
        let result = async {
            let dir = deck::deck_dir(app, id)?;
            let frozen_root = root.clone();
            let flag = cancelled.clone();
            let job = job_id.to_string();
            let deck = id.to_string();
            let timeline = tauri::async_runtime::spawn_blocking(move || {
                freeze(&dir, &frozen_root, &job, &deck, &flag)
            })
            .await
            .map_err(|e| Error::msg(e.to_string()))??;
            run(app, &root, job_id, "render", None, &cancelled).await?;
            for index in 0..timeline.slides.len() {
                let image = crate::capture::decode_png(&fs::read(
                    root.join(format!("frame-{index}.png")),
                )?)?;
                if image.width != 1920 || image.height != 1080 {
                    return Err(Error::msg("Rendered frame is not 1080p."));
                }
            }
            self.jobs
                .lock()
                .expect("video mutex")
                .get_mut(job_id)
                .expect("reserved job")
                .timeline = Some(timeline.clone());
            Ok(timeline)
        }
        .await;
        if result.is_err() {
            self.jobs.lock().expect("video mutex").remove(job_id);
            let _ = fs::remove_dir_all(root);
        }
        result
    }
    pub async fn export(&self, app: &AppHandle, id: &str, dest: &Path) -> Result<()> {
        let _active = self.acquire(id)?;
        let job = self
            .jobs
            .lock()
            .expect("video mutex")
            .get(id)
            .filter(|j| j.timeline.is_some())
            .cloned()
            .ok_or_else(|| Error::msg("Prepare a narrated preview first."))?;
        cancelled(&job.cancelled)?;
        let output = Output::new(dest)?;
        let temp = &output.temp;
        let result = async {
            run(app, &job.root, id, "encode", Some(temp), &job.cancelled).await?;
            cancelled(&job.cancelled)?;
            publish(temp, dest)?;
            Ok(())
        }
        .await;
        if job.cancelled.load(Ordering::Relaxed) {
            self.jobs.lock().expect("video mutex").remove(id);
            let _ = fs::remove_dir_all(job.root);
        }
        result
    }
}
struct Output {
    temp: PathBuf,
}
impl Output {
    fn new(dest: &Path) -> Result<Self> {
        let temp = destination_temp(dest)?;
        fs::create_dir(temp.parent().expect("temporary output parent"))?;
        Ok(Self { temp })
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.temp.parent().expect("temporary output parent"));
    }
}
fn destination_temp(dest: &Path) -> Result<PathBuf> {
    if !dest.is_absolute()
        || dest
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            != Some("mp4".into())
    {
        return Err(Error::msg("Choose an absolute .mp4 output path."));
    }
    if fs::symlink_metadata(dest).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) {
        return Err(Error::msg("MP4 output must be a regular file."));
    }
    let parent = dest
        .parent()
        .filter(|p| p.is_dir())
        .ok_or_else(|| Error::msg("Output folder does not exist."))?;
    Ok(parent
        .join(format!(".slopslide-video-{}", uuid::Uuid::new_v4()))
        .join("movie.mp4"))
}
fn publish(temp: &Path, dest: &Path) -> Result<()> {
    if fs::metadata(temp)?.len() == 0 {
        return Err(Error::msg("Encoder returned an empty movie."));
    }
    fs::File::open(temp)?.sync_all()?;
    fs::rename(temp, dest)?;
    Ok(())
}
async fn run(
    app: &AppHandle,
    root: &Path,
    id: &str,
    operation: &str,
    dest: Option<&Path>,
    flag: &AtomicBool,
) -> Result<()> {
    let helper = app
        .path()
        .resource_dir()
        .map_err(|e| Error::msg(e.to_string()))?
        .join("video-runtime/slopslide-video");
    let mut command = tokio::process::Command::new(helper);
    command
        .arg(operation)
        .arg(root)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(dest) = dest {
        command.arg(dest);
    }
    let mut child = command
        .spawn()
        .map_err(|e| Error::msg(format!("Could not start bundled video helper: {e}")))?;
    let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
    let stderr = child.stderr.take().expect("piped");
    let errors = tokio::spawn(async move {
        let mut text = String::new();
        let mut stderr = stderr;
        let mut buffer = [0u8; 4096];
        while let Ok(count) = stderr.read(&mut buffer).await {
            if count == 0 {
                break;
            }
            text.push_str(&String::from_utf8_lossy(&buffer[..count]));
            if text.len() > 65_536 {
                text = text
                    .chars()
                    .rev()
                    .take(32_768)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
            }
        }
        text
    });
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(if operation == "render" { 180 } else { 3600 });
    loop {
        tokio::select! {
            line = lines.next_line() => match line? {
                Some(line) => if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                    let event = Progress { id: id.into(), stage: value["stage"].as_str().unwrap_or(operation).into(), completed: value["completed"].as_u64().unwrap_or(0), total: value["total"].as_u64().unwrap_or(0) };
                    let _ = app.emit("video-progress", event);
                },
                None => break,
            },
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                if flag.load(Ordering::Relaxed) || tokio::time::Instant::now() >= deadline {
                    let _ = child.kill().await; let _ = child.wait().await; errors.abort();
                    return Err(Error::msg(if flag.load(Ordering::Relaxed) { "Video job cancelled." } else { "Video helper timed out." }));
                }
            }
        }
    }
    let status = child.wait().await?;
    let errors = errors.await.unwrap_or_default();
    if !status.success() {
        return Err(Error::msg(format!(
            "Video {operation} failed: {}",
            errors
                .chars()
                .rev()
                .take(2000)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        )));
    }
    cancelled(flag)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("video-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn html(ids: &[&str]) -> String {
        html::ensure_runtime(&format!("<!DOCTYPE html><html><head><title>Video test</title><style>.slide {{ background: #a02030; color:white; font:80px Helvetica }} #b {{ background:#2040b0 }}</style></head><body><main class=\"deck\">{}</main></body></html>", ids.iter().map(|id| format!("<section class=\"slide\" id=\"{id}\"><h1 class=\"reveal\">{id}</h1></section>")).collect::<String>()))
    }
    fn silent(ids: &[&str], ms: u32) -> narration::Manifest {
        let mut m = narration::Manifest::default();
        for id in ids {
            m.slides.insert(
                (*id).into(),
                narration::SlideNarration {
                    silent_duration_ms: Some(ms),
                    ..Default::default()
                },
            );
        }
        m
    }
    fn save(dir: &Path, source: &str, manifest: &narration::Manifest) {
        fs::write(dir.join("deck.html"), source).unwrap();
        fs::write(
            dir.join("narration.json"),
            serde_json::to_vec(manifest).unwrap(),
        )
        .unwrap();
    }
    fn spoken(dir: &Path, m: &mut narration::Manifest, id: &str, pcm: &[i16]) -> cache::Take {
        m.slides.insert(
            id.into(),
            narration::SlideNarration {
                text: "Test speech".into(),
                ..Default::default()
            },
        );
        let source = cache::Source::from_manifest(m, id).unwrap();
        let provider = speech_connector::FixtureProvider.describe();
        let take = cache::publish_for(dir, source, pcm, &provider).unwrap();
        m.slides.get_mut(id).unwrap().accepted_take_id = Some(take.id.clone());
        take
    }
    use speech_connector::SpeechProvider;
    #[test]
    fn cumulative_rounding_does_not_drift_and_final_audio_is_padded() {
        let dir = Temp::new();
        let ids: Vec<String> = (0..1000).map(|i| format!("s{i}")).collect();
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let t = timeline(&html(&refs), &silent(&refs, 101), &dir.0, "job", "deck").unwrap();
        assert_eq!(t.total_frames, 3030);
        assert_eq!(t.total_samples, 101 * RATE);
        for s in &t.slides {
            assert!(s.start_frame.abs_diff(s.start_sample / FRAME_SAMPLES) <= 1);
            assert!(s.end_frame.abs_diff(s.end_sample / FRAME_SAMPLES) <= 1);
        }
        let t = timeline(&html(&["a"]), &silent(&["a"], 101), &dir.0, "job", "deck").unwrap();
        assert_eq!(t.total_frames, 4);
        assert_eq!(t.total_samples, 3200);
    }
    #[test]
    fn hidden_and_removed_slides_are_excluded_and_document_order_is_preserved() {
        let dir = Temp::new();
        let source = html(&["b", "a"]).replace("id=\"a\"", "id=\"a\" data-hidden");
        let t = timeline(
            &source,
            &silent(&["a", "b", "removed"], 1000),
            &dir.0,
            "job",
            "deck",
        )
        .unwrap();
        assert_eq!(t.slides.len(), 1);
        assert_eq!(t.slides[0].id, "b");
        assert_eq!(t.total_frames, 30);
    }
    #[test]
    fn missing_and_stale_audio_blocks_instead_of_guessing_speech_duration() {
        let dir = Temp::new();
        let mut m = silent(&["a"], 1000);
        m.slides.get_mut("a").unwrap().text = "Test speech".into();
        assert!(timeline(&html(&["a"]), &m, &dir.0, "job", "deck")
            .unwrap_err()
            .to_string()
            .contains("generate"));
        spoken(&dir.0, &mut m, "a", &[1; 24_000]);
        assert!(timeline(&html(&["a"]), &m, &dir.0, "job", "deck").is_ok());
        m.slides.get_mut("a").unwrap().text = "Different speech".into();
        assert!(timeline(&html(&["a"]), &m, &dir.0, "job", "deck")
            .unwrap_err()
            .to_string()
            .contains("regenerate"));
    }
    #[test]
    fn audio_sample_counts_and_pauses_match_preview_and_movie_timeline() {
        let dir = Temp::new();
        let root = Temp::new();
        let mut m = silent(&["a", "b"], 1000);
        spoken(&dir.0, &mut m, "a", &[1234; 24_001]);
        save(&dir.0, &html(&["a", "b"]), &m);
        let t = freeze(&dir.0, &root.0, "job", "deck", &AtomicBool::new(false)).unwrap();
        assert_eq!(t.slides[0].end_sample, 42_001);
        assert_eq!(t.slides[1].start_sample, 42_001);
        let pcm = cache::decode_wav(&fs::read(root.0.join("timeline.wav")).unwrap()).unwrap();
        assert_eq!(pcm.len() as u64, t.total_samples);
        assert!(pcm[..6000].iter().all(|s| *s == 0));
        assert!(pcm[6000..30_001].iter().all(|s| *s == 1234));
        assert!(pcm[30_001..].iter().all(|s| *s == 0));
    }
    #[test]
    fn consecutive_spoken_slides_keep_both_takes_and_their_boundary_silence() {
        let dir = Temp::new();
        let root = Temp::new();
        let mut m = silent(&["a", "b"], 1000);
        spoken(&dir.0, &mut m, "a", &[1234; 24001]);
        spoken(&dir.0, &mut m, "b", &[-2345; 12003]);
        m.slides.get_mut("b").unwrap().lead_in_ms = 100;
        m.slides.get_mut("b").unwrap().tail_ms = 200;
        save(&dir.0, &html(&["a", "b"]), &m);
        let t = freeze(&dir.0, &root.0, "job", "deck", &AtomicBool::new(false)).unwrap();
        let pcm = cache::decode_wav(&fs::read(root.0.join("timeline.wav")).unwrap()).unwrap();
        assert_eq!(t.slides[1].audio_start_sample, 44401);
        assert!(pcm[6000..30001].iter().all(|s| *s == 1234));
        assert!(pcm[30001..44401].iter().all(|s| *s == 0));
        assert!(pcm[44401..56404].iter().all(|s| *s == -2345));
        assert!(pcm[56404..].iter().all(|s| *s == 0));
        assert_eq!(pcm.len() as u64, t.total_samples);
    }
    #[test]
    fn slide_voice_changes_keep_other_recordings_exportable_and_restoration_reuses_audio() {
        let dir = Temp::new();
        let root = Temp::new();
        let mut m = silent(&["a", "b"], 1000);
        let a = spoken(&dir.0, &mut m, "a", &[1234; 24000]);
        spoken(&dir.0, &mut m, "b", &[2345; 12000]);
        let b = m.slides.get_mut("b").unwrap();
        b.speech_provider_id_override = Some("fixture-tone".into());
        b.presenter_id_override = Some("tone:440".into());
        b.pace_override = Some(1.5);
        assert_eq!(cache::Source::from_manifest(&m, "a").unwrap(), a.source);
        assert!(timeline(&html(&["a", "b"]), &m, &dir.0, "job", "deck")
            .unwrap_err()
            .to_string()
            .contains("Slide 2"));
        let take_b = cache::publish_for(
            &dir.0,
            cache::Source::from_manifest(&m, "b").unwrap(),
            &[-2345; 12000],
            &speech_connector::FixtureProvider.describe(),
        )
        .unwrap();
        m.slides.get_mut("b").unwrap().accepted_take_id = Some(take_b.id.clone());
        save(&dir.0, &html(&["a", "b"]), &m);
        let t = freeze(&dir.0, &root.0, "job", "deck", &AtomicBool::new(false)).unwrap();
        assert_eq!(t.slides[0].take_id.as_deref(), Some(a.id.as_str()));
        assert_eq!(t.slides[1].take_id.as_deref(), Some(take_b.id.as_str()));
        let pcm = cache::decode_wav(&fs::read(root.0.join("timeline.wav")).unwrap()).unwrap();
        assert!(pcm[6000..30000].iter().all(|s| *s == 1234));
        // A deliberate deck-default edit can invalidate inherited A. Restoring
        // its recorded settings pins A and reuses its original immutable WAV.
        m.pace = 1.2;
        m.presenter_id = "preset:aiden".into();
        assert!(timeline(&html(&["a", "b"]), &m, &dir.0, "job", "deck").is_err());
        let s = m.slides.get_mut("a").unwrap();
        s.pace_override = Some(a.source.pace);
        s.presenter_id_override = Some(a.source.presenter_id.clone());
        s.speech_provider_id_override = Some(a.source.provider_id.clone());
        assert!(timeline(&html(&["a", "b"]), &m, &dir.0, "job", "deck").is_ok());
        assert_eq!(
            cache::read(&dir.0, &a.id).unwrap().unwrap().sha256,
            a.sha256
        );
    }
    #[test]
    fn prepared_copy_survives_simultaneous_source_edits() {
        let dir = Temp::new();
        let root = Temp::new();
        let source = html(&["a"]);
        save(&dir.0, &source, &silent(&["a"], 1000));
        fs::create_dir(dir.0.join("assets")).unwrap();
        fs::write(dir.0.join("assets/example.txt"), "before").unwrap();
        freeze(&dir.0, &root.0, "job", "deck", &AtomicBool::new(false)).unwrap();
        fs::write(dir.0.join("deck.html"), "modified").unwrap();
        fs::write(dir.0.join("assets/example.txt"), "after").unwrap();
        assert_eq!(
            fs::read_to_string(root.0.join("deck.html")).unwrap(),
            source
        );
        assert_eq!(
            fs::read_to_string(root.0.join("assets/example.txt")).unwrap(),
            "before"
        );
    }
    #[test]
    fn cancelled_freeze_stops_and_failed_publication_preserves_existing_movie() {
        let dir = Temp::new();
        let root = Temp::new();
        save(&dir.0, &html(&["a"]), &silent(&["a"], 1000));
        assert!(freeze(&dir.0, &root.0, "job", "deck", &AtomicBool::new(true)).is_err());
        let dest = root.0.join("existing.mp4");
        fs::write(&dest, "existing").unwrap();
        assert!(publish(&root.0.join("absent.mp4"), &dest).is_err());
        assert_eq!(fs::read_to_string(&dest).unwrap(), "existing");
        let empty = root.0.join("empty.mp4");
        fs::write(&empty, []).unwrap();
        assert!(publish(&empty, &dest).is_err());
        assert_eq!(fs::read_to_string(&dest).unwrap(), "existing");
        assert!(destination_temp(Path::new("relative.mp4")).is_err());
        assert!(destination_temp(&root.0.join("movie.html")).is_err());
    }
    #[test]
    fn video_protocol_exposes_only_ready_pngs_and_timeline_audio() {
        let manager = VideoManager::default();
        let dir = Temp::new();
        let t = timeline(&html(&["a"]), &silent(&["a"], 1000), &dir.0, "job", "deck").unwrap();
        manager.jobs.lock().unwrap().insert(
            "job".into(),
            Job {
                root: dir.0.clone(),
                cancelled: Arc::new(AtomicBool::new(false)),
                timeline: Some(t),
            },
        );
        assert!(manager.file("job", "frame-0.png").is_some());
        assert!(manager.file("job", "timeline.wav").is_some());
        for name in [
            "frame-1.png",
            "frame-00.png",
            "../deck.html",
            "job.json",
            "frozen.html",
            "frame-0.png/../job.json",
        ] {
            assert!(manager.file("job", name).is_none());
        }
    }
    #[cfg(unix)]
    #[test]
    fn asset_symlinks_are_rejected() {
        let dir = Temp::new();
        let root = Temp::new();
        save(&dir.0, &html(&["a"]), &silent(&["a"], 1000));
        fs::create_dir(dir.0.join("assets")).unwrap();
        std::os::unix::fs::symlink("/tmp", dir.0.join("assets/escape")).unwrap();
        assert!(
            freeze(&dir.0, &root.0, "job", "deck", &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("symbolic")
        );
    }
    #[test]
    fn changing_an_otherwise_valid_take_after_timeline_creation_is_rejected() {
        let dir = Temp::new();
        let mut m = silent(&["a"], 1000);
        let mut take = spoken(&dir.0, &mut m, "a", &[1234; 24000]);
        let t = timeline(&html(&["a"]), &m, &dir.0, "job", "deck").unwrap();
        let wav = cache::encode_wav(&[4321; 24000]);
        take.sha256 = hash(&wav);
        fs::write(cache::take_path(&dir.0, &take.id, "wav").unwrap(), wav).unwrap();
        fs::write(
            cache::take_path(&dir.0, &take.id, "json").unwrap(),
            serde_json::to_vec(&take).unwrap(),
        )
        .unwrap();
        assert!(cache::read(&dir.0, &take.id).unwrap().is_some());
        let error = stream_audio(&dir.0, &t, &AtomicBool::new(false), &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("changed while freezing"));
    }
    #[test]
    fn disk_full_during_audio_write_returns_error_without_touching_accepted_recordings() {
        struct Full;
        impl std::io::Write for Full {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from_raw_os_error(28))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let dir = Temp::new();
        let mut m = silent(&["a"], 1000);
        let take = spoken(&dir.0, &mut m, "a", &[1234; 24000]);
        let t = timeline(&html(&["a"]), &m, &dir.0, "job", "deck").unwrap();
        assert!(stream_audio(&dir.0, &t, &AtomicBool::new(false), &mut Full).is_err());
        assert_eq!(
            cache::read(&dir.0, &take.id).unwrap().unwrap().sha256,
            take.sha256
        );
        let dest = dir.0.join("existing.mp4");
        fs::write(&dest, "existing movie").unwrap();
        let output = Output::new(&dest).unwrap();
        let scratch = output.temp.parent().unwrap().to_path_buf();
        fs::write(&output.temp, "partial output").unwrap();
        drop(output);
        assert!(!scratch.exists());
        assert_eq!(fs::read_to_string(dest).unwrap(), "existing movie");
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "opt-in native 10-minute WebKit/AVFoundation qualification; pnpm video:build first"]
    fn native_ten_minute_render_and_encode() {
        let dir = Temp::new();
        let root = Temp::new();
        let mut m = silent(&["a", "b"], 600_000);
        let wav = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../docs/feasibility/listening-2026-10-09/b-native-bf16-ryan.wav"),
        )
        .unwrap();
        let pcm = cache::decode_wav(&wav).unwrap();
        spoken(&dir.0, &mut m, "a", &pcm);
        let speech_ms = ((pcm.len() as u64 + 18_000) * 1000 / RATE) as u32;
        m.slides.get_mut("b").unwrap().silent_duration_ms = Some(600_000 - speech_ms);
        save(&dir.0, &html(&["a", "b"]), &m);
        let t = freeze(&dir.0, &root.0, "job", "deck", &AtomicBool::new(false)).unwrap();
        assert!(t.total_frames.abs_diff(18_000) <= 1);
        let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("video-runtime/slopslide-video");
        for mode in ["render", "encode"] {
            let mut cmd = std::process::Command::new(&helper);
            cmd.arg(mode).arg(&root.0);
            if mode == "encode" {
                cmd.arg(root.0.join("result.mp4"));
            }
            let output = cmd.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        for (index, color) in [(0, [160, 32, 48]), (1, [32, 64, 176])] {
            let image = crate::capture::decode_png(
                &fs::read(root.0.join(format!("frame-{index}.png"))).unwrap(),
            )
            .unwrap();
            assert_eq!((image.width, image.height), (1920, 1080));
            let p = (540 * 1920 + 960) * 4;
            assert_eq!(&image.rgba[p..p + 3], &color);
        }
        if let Ok(path) = std::env::var("SLOPSLIDE_VIDEO_EVIDENCE") {
            fs::create_dir_all(&path).unwrap();
            for name in [
                "result.mp4",
                "job.json",
                "frame-0.png",
                "frame-1.png",
                "timeline.wav",
            ] {
                fs::copy(root.0.join(name), Path::new(&path).join(name)).unwrap();
            }
        }
    }
}
