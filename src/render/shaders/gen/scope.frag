// Audio scope: a vectorscope (stereo XY / Lissajous) or a waveform line, drawn
// from the live waveform texture (u_wave: R = L sample, G = R sample, 0..1).
// Param 1 picks the mode, Param 2 the line thickness/glow.
void main() {
    vec2 uv = av_coord();
    float d = 1e9;

    if (u_p1 < 0.5) {
        // Vectorscope: plot the stereo signal, rotated 45 deg so mono is a
        // vertical line and stereo width opens it out (classic audio 'scope).
        for (int i = 0; i < 128; i++) {
            vec2 s = TEX2D(u_wave, vec2((float(i) + 0.5) / 128.0, 0.5)).rg * 2.0 - 1.0;
            vec2 p = vec2(s.x - s.y, s.x + s.y) * 0.6;
            d = min(d, length(uv - p));
        }
    } else {
        // Waveform: x is time, y is the (left) sample.
        float x = uv.x * 0.5 + 0.5;
        float s = TEX2D(u_wave, vec2(x, 0.5)).r * 2.0 - 1.0;
        d = abs(uv.y - s * 0.8);
    }

    float thick = mix(0.012, 0.05, u_p2);
    float line = smoothstep(thick, 0.0, d);
    float glow = thick * 0.6 / (d + thick * 0.6);
    vec3 col = av_hsv(u_hue, 0.6, 1.0) * (line + glow * (0.4 + u_audio.x));
    FRAG_COLOR = vec4(col, 1.0);
}
