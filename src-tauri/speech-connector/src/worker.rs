use crate::SynthesisRequest;
use crate::{Error, Result};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout},
    sync::watch,
};

pub struct Worker {
    child: Child,
    input: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    _pack_lock: super::models::PackLock,
}
impl Worker {
    #[cfg(test)]
    pub async fn start(
        bin: &Path,
        root: &Path,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Self> {
        Self::start_voice(bin, root, super::models::Kind::CustomVoice, None, cancel).await
    }
    pub async fn start_voice(
        bin: &Path,
        root: &Path,
        kind: super::models::Kind,
        profile: Option<&Path>,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Self> {
        let pack_lock = super::models::lock(root, true)?;
        let mut command = tokio::process::Command::new(bin);
        command.arg(kind.location(root));
        if let Some(profile) = profile {
            command.arg(profile);
        }
        let mut child = command
            .env("QWEN_NO_KLEIDI", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::msg(format!("Could not start local speech: {e}")))?;
        let mut worker = Self {
            input: child.stdin.take().expect("stdin"),
            lines: BufReader::new(child.stdout.take().expect("stdout")).lines(),
            child,
            _pack_lock: pack_lock,
        };
        let ready = worker.next(cancel).await?;
        if ready["type"] != "ready" {
            return Err(Error::msg("Speech worker did not become ready."));
        }
        Ok(worker)
    }
    async fn next(&mut self, cancel: &mut watch::Receiver<bool>) -> Result<Value> {
        if *cancel.borrow() {
            return Err(Error::msg("Audio generation cancelled."));
        }
        tokio::select! {
            _ = cancel.changed() => Err(Error::msg("Audio generation cancelled.")),
            result = tokio::time::timeout(Duration::from_secs(180), async {
                loop {
                    let line = self.lines.next_line().await?.ok_or_else(|| Error::msg("Local speech stopped unexpectedly. Retry generation; your previous recording is kept."))?;
                    if line.len() > 65_536 { return Err(Error::msg("Oversized speech worker message.")); }
                    let Some(frame) = line.strip_prefix("SLOPSPEECH ") else { continue; };
                    let event: Value = serde_json::from_str(frame).map_err(|e| Error::msg(e.to_string()))?;
                    if event["version"] != 1 { return Err(Error::msg("Unsupported speech worker version.")); }
                    if event["type"] == "error" { return Err(Error::msg(event["message"].as_str().unwrap_or("Local speech failed."))); }
                    return Ok(event);
                }
            }) => result.map_err(|_| Error::msg("Local speech timed out. Retry generation."))?,
        }
    }
    pub async fn generate(
        &mut self,
        source: &SynthesisRequest,
        text: &str,
        output: &Path,
        cancel: &mut watch::Receiver<bool>,
        progress: &(dyn Fn(u64) + Sync),
    ) -> Result<usize> {
        let request = json!({ "text":text, "speaker":source.voice_id.strip_prefix("preset:").unwrap_or("saved"), "language": if source.language == "de" { "German" } else { "English" }, "pace":format!("{:.2}", source.pace), "output":output });
        let mut bytes = serde_json::to_vec(&request).expect("speech JSON");
        bytes.push(b'\n');
        if bytes.len() > 32767 {
            return Err(Error::msg("Speech request is too large."));
        }
        self.input.write_all(&bytes).await?;
        self.input.flush().await?;
        loop {
            let frame = self.next(cancel).await?;
            match frame["type"].as_str() {
                Some("progress") => progress(frame["frames"].as_u64().unwrap_or(0)),
                Some("done") => {
                    return frame["samples"]
                        .as_u64()
                        .filter(|n| *n > 0 && *n <= 50 * 24000)
                        .map(|n| n as usize)
                        .ok_or_else(|| Error::msg("Invalid audio sample count."))
                }
                _ => return Err(Error::msg("Unexpected speech worker message.")),
            }
        }
    }
    pub async fn stop(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn rejects_wrong_protocol_and_kills_a_cancelled_worker() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let dir=std::env::temp_dir().join(format!("speech-worker-{}",uuid::Uuid::new_v4())); std::fs::create_dir_all(&dir).unwrap();
            let bin=dir.join("worker");
            std::fs::write(&bin,"#!/bin/sh\necho 'SLOPSPEECH {\"version\":9,\"type\":\"ready\"}'\n").unwrap();
            std::fs::set_permissions(&bin,std::fs::Permissions::from_mode(0o755)).unwrap();
            let (signal,mut cancel)=watch::channel(false);
            assert!(Worker::start(&bin,&dir,&mut cancel).await.err().unwrap().to_string().contains("version"));
            std::fs::write(&bin,"#!/bin/sh\necho 'SLOPSPEECH {\"version\":1,\"type\":\"ready\"}'\nwhile read line; do :; done\n").unwrap();
            let mut worker=Worker::start(&bin,&dir,&mut cancel).await.unwrap();
            signal.send(true).unwrap();
            let source=SynthesisRequest{text:"Hello".into(),language:"en".into(),voice_id:"preset:ryan".into(),voice_revision: None, pace:1.1};
            assert!(worker.generate(&source,"Hello",&dir.join("out.wav"),&mut cancel,&|_|{}).await.is_err());
            worker.stop().await; assert!(worker.child.try_wait().unwrap().is_some()); assert!(!dir.join("out.wav").exists()); std::fs::remove_dir_all(dir).unwrap();
        });
    }
}
