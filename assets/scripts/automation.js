// Drive the show from audio + an LFO. Works with any generator on Layer 1.
// Available each frame: t, dt, frame, low, mid, high, rms, onset, beat, bar,
// bpm, lfo[0..5]. Use set(path,value), setn(path,0..1), get(path), trigger(path).

set("layer.0.hue", (0.5 + 0.5 * Math.sin(t * 0.2)) % 1.0);
set("layer.0.speed", 0.4 + low * 3.0);
set("post.feedback.amount", 0.35 + high * 0.5);
set("global.brightness", 0.7 + rms * 0.3);
