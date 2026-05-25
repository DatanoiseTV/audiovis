// Strobe the master on detected onsets and cycle hue with the bar phase.
set("global.brightness", onset > 0.6 ? 1.0 : 0.22);
set("layer.0.hue", bar);
set("layer.0.scale", 1.0 + lfo[0] * 0.6);
