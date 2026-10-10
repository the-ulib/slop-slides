import { afterEach, describe, expect, it, vi } from "vitest";
import { pcmWav, startVoiceRecording } from "./voiceRecording";
afterEach(() => vi.unstubAllGlobals());
describe("microphone capture", () => {
  it("releases the microphone/context if audio setup fails", async () => {
    const stop = vi.fn(), close = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: vi.fn().mockResolvedValue({ getTracks: () => [{ stop }] }) } });
    vi.stubGlobal("AudioContext", class { resume = vi.fn().mockResolvedValue(undefined); close = close; createMediaStreamSource() { throw new Error("Capture setup failed"); } });
    await expect(startVoiceRecording(vi.fn())).rejects.toThrow("Capture setup failed");
    expect(stop).toHaveBeenCalledOnce(); expect(close).toHaveBeenCalledOnce();
  });
  it("stops capture at thirty seconds and makes stop/cancel idempotent", async () => {
    const stop = vi.fn(), close = vi.fn().mockResolvedValue(undefined), onLimit = vi.fn();
    const processor = { disconnect: vi.fn(), connect: vi.fn(), onaudioprocess: null as null | ((e: { inputBuffer: { getChannelData: () => Float32Array } }) => void) };
    const source = { disconnect: vi.fn(), connect: vi.fn() };
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: vi.fn().mockResolvedValue({ getTracks: () => [{ stop }] }) } });
    vi.stubGlobal("AudioContext", class { sampleRate = 8000; resume = vi.fn().mockResolvedValue(undefined); close = close; createMediaStreamSource() { return source; } createScriptProcessor() { return processor; } destination = {}; });
    const capture = await startVoiceRecording(onLimit);
    processor.onaudioprocess!({ inputBuffer: { getChannelData: () => new Float32Array(8000 * 31) } });
    const wave = await capture.stop(); capture.cancel();
    expect(wave.length).toBe(44 + 8000 * 30 * 2); expect(onLimit).toHaveBeenCalledOnce(); expect(stop).toHaveBeenCalledOnce(); expect(close).toHaveBeenCalledOnce(); expect(processor.onaudioprocess).toBeNull();
  });
  it("discards exactly the spoken warm-up across chunk boundaries, keeping thirty seconds of reference", async () => {
    const stop = vi.fn(), onReady = vi.fn(), onLimit = vi.fn();
    const processor = { disconnect: vi.fn(), connect: vi.fn(), onaudioprocess: null as null | ((e: { inputBuffer: { getChannelData: () => Float32Array } }) => void) };
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: vi.fn().mockResolvedValue({ getTracks: () => [{ stop }] }) } });
    vi.stubGlobal("AudioContext", class { sampleRate = 8000; resume = vi.fn().mockResolvedValue(undefined); close = vi.fn().mockResolvedValue(undefined); createMediaStreamSource() { return { connect: vi.fn(), disconnect: vi.fn() }; } createScriptProcessor() { return processor; } destination = {}; });
    const capture = await startVoiceRecording(onLimit, { warmupSeconds: 5, onReady });
    const feed = (samples: Float32Array) => processor.onaudioprocess!({ inputBuffer: { getChannelData: () => samples } });
    feed(new Float32Array(8000 * 4).fill(0.1)); expect(onReady).not.toHaveBeenCalled();
    const boundary = new Float32Array(8000 * 2).fill(0.5); boundary.fill(0.1, 0, 8000);
    feed(boundary); expect(onReady).toHaveBeenCalledOnce(); expect(onLimit).not.toHaveBeenCalled();
    feed(new Float32Array(8000 * 30).fill(0.5));
    const wave = await capture.stop();
    const view = new DataView(wave.buffer as ArrayBuffer);
    expect(wave.length).toBe(44 + 8000 * 30 * 2);
    const samples = Array.from({ length: (wave.length - 44) / 2 }, (_, n) => view.getInt16(44 + n * 2, true));
    expect(samples.every((sample) => sample === 16384)).toBe(true);
    expect(onReady).toHaveBeenCalledOnce(); expect(onLimit).toHaveBeenCalledOnce(); expect(stop).toHaveBeenCalledOnce();
  });
  it("releases capture and rejects empty reference when stopped during warm-up", async () => {
    const stop = vi.fn(), onReady = vi.fn();
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia: vi.fn().mockResolvedValue({ getTracks: () => [{ stop }] }) } });
    vi.stubGlobal("AudioContext", class { sampleRate = 8000; resume = vi.fn().mockResolvedValue(undefined); close = vi.fn().mockResolvedValue(undefined); createMediaStreamSource() { return { connect: vi.fn(), disconnect: vi.fn() }; } createScriptProcessor() { return { connect: vi.fn(), disconnect: vi.fn(), onaudioprocess: null }; } destination = {}; });
    const capture = await startVoiceRecording(vi.fn(), { warmupSeconds: 5, onReady });
    await expect(capture.stop()).rejects.toThrow("No reference speech"); capture.cancel();
    expect(stop).toHaveBeenCalledOnce(); expect(onReady).not.toHaveBeenCalled();
    await expect(startVoiceRecording(vi.fn(), { warmupSeconds: NaN })).rejects.toThrow("Invalid microphone warm-up");
    expect(navigator.mediaDevices.getUserMedia).toHaveBeenCalledOnce();
  });
  it("encodes actual samples as bounded mono PCM16 WAV and clamps invalid amplitudes", () => {
    const wave=pcmWav([new Float32Array([0,1,-1,2,NaN]),new Float32Array([0.5])],48000);
    const view=new DataView(wave.buffer as ArrayBuffer);
    expect(new TextDecoder().decode(wave.slice(0,4))).toBe("RIFF");expect(view.getUint32(24,true)).toBe(48000);
    expect(view.getUint32(40,true)).toBe(12);expect(view.getInt16(46,true)).toBe(32767);expect(view.getInt16(48,true)).toBe(-32767);expect(view.getInt16(52,true)).toBe(0);
    expect(() => pcmWav([new Float32Array(8000*31)],8000)).toThrow("30-second");
  });
  it("surfaces unavailable/denied microphone access without starting an audio context", async () => {
    const original=navigator.mediaDevices;
    Object.defineProperty(navigator,"mediaDevices",{configurable:true,value:undefined});
    await expect(startVoiceRecording(vi.fn())).rejects.toThrow("Import a WAV");
    const request=vi.fn().mockRejectedValue(new Error("Permission denied"));
    Object.defineProperty(navigator,"mediaDevices",{configurable:true,value:{getUserMedia:request}});
    await expect(startVoiceRecording(vi.fn())).rejects.toThrow("Permission denied");
    expect(request).toHaveBeenCalledWith({audio:expect.objectContaining({channelCount:1,echoCancellation:false})});
    Object.defineProperty(navigator,"mediaDevices",{configurable:true,value:original});
  });
});
