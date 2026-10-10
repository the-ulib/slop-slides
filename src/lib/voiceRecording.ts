/** A bounded PCM16 microphone recording; no network or automatic transcription. */
export interface VoiceRecording { stop: () => Promise<Uint8Array>; cancel: () => void }
export function pcmWav(chunks: Float32Array[], sampleRate: number): Uint8Array {
  const count = chunks.reduce((n, c) => n + c.length, 0);
  if (count > 30 * sampleRate || sampleRate < 8000 || sampleRate > 96000) throw new Error("Recording exceeds the 30-second limit.");
  const buffer = new ArrayBuffer(44 + count * 2), view = new DataView(buffer);
  const text = (at: number, value: string) => { for (let n = 0; n < value.length; n++) view.setUint8(at + n, value.charCodeAt(n)); };
  text(0, "RIFF"); view.setUint32(4, 36 + count * 2, true); text(8, "WAVEfmt ");
  view.setUint32(16, 16, true); view.setUint16(20, 1, true); view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true); view.setUint32(28, sampleRate * 2, true); view.setUint16(32, 2, true); view.setUint16(34, 16, true);
  text(36, "data"); view.setUint32(40, count * 2, true);
  let at = 44;
  for (const chunk of chunks) for (const value of chunk) { view.setInt16(at, Math.round(Math.max(-1, Math.min(1, Number.isFinite(value) ? value : 0)) * 32767), true); at += 2; }
  return new Uint8Array(buffer);
}
export async function startVoiceRecording(onLimit: () => void): Promise<VoiceRecording> {
  if (!navigator.mediaDevices?.getUserMedia) throw new Error("Microphone recording is unavailable in this build. Import a WAV recording instead.");
  const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: false, noiseSuppression: false, autoGainControl: false } });
  let context: AudioContext | undefined, source: MediaStreamAudioSourceNode | undefined, processor: ScriptProcessorNode | undefined;
  const chunks: Float32Array[] = []; let count = 0, stopped = false;
  const close = () => { if (stopped) return; stopped = true; if (processor) { processor.onaudioprocess = null; processor.disconnect(); } source?.disconnect(); stream.getTracks().forEach((t) => t.stop()); if (context) void context.close().catch(() => {}); };
  try {
    context = new AudioContext(); await context.resume();
    source = context.createMediaStreamSource(stream);
    // Bounded PoC capture; an AudioWorklet can replace this without changing its output contract.
    processor = context.createScriptProcessor(4096, 1, 1);
    source.connect(processor); processor.connect(context.destination);
  } catch (error) { close(); throw error; }
  const sampleRate = context.sampleRate;
  processor.onaudioprocess = (event) => {
    if (stopped) return;
    const input = event.inputBuffer.getChannelData(0);
    const remaining = 30 * sampleRate - count;
    if (remaining > 0) { const part = input.slice(0, remaining); chunks.push(part); count += part.length; }
    if (count >= 30 * sampleRate) { close(); onLimit(); }
  };
  return { stop: async () => { close(); return pcmWav(chunks, sampleRate); }, cancel: close };
}
