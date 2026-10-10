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
