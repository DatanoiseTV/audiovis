// Analog / VHS look in a single GLES2-friendly pass. Every effect is scaled by
// its own uniform so it can be dialled from nothing to wrecked, and the whole
// pass is skipped when disabled. Prepended with lib.glsl (hash/noise, u_time,
// u_res, v_uv).
uniform sampler2D u_tex;
uniform float u_aberration; // RGB split amount
uniform float u_bleed;      // horizontal chroma smear
uniform float u_scan;       // scanline depth
uniform float u_noise;      // tape grain
uniform float u_wobble;     // tracking / head-switching jitter
uniform float u_vignette;   // edge darkening
uniform float u_sat;        // saturation grade (-1..+1)

// Average a few left-hand taps to fake the smeared chroma of composite video.
vec3 sample_bleed(vec2 uv) {
    vec3 c = vec3(0.0);
    float w = 0.0;
    for (int i = 0; i < 4; i++) {
        float o = float(i) * 0.004 * u_bleed;
        float k = 1.0 - float(i) * 0.2;
        c += TEX2D(u_tex, uv - vec2(o, 0.0)).rgb * k;
        w += k;
    }
    return c / w;
}

void main() {
    vec2 uv = v_uv;

    // Tape wobble: a smooth horizontal sway plus per-line tracking jitter.
    float jitter = av_hash(vec2(floor(uv.y * u_res.y), floor(u_time * 15.0))) - 0.5;
    uv.x += u_wobble * (0.01 * sin(uv.y * 30.0 + u_time * 5.0) + 0.02 * jitter);

    // Chromatic aberration: split the channels horizontally.
    float ab = u_aberration * 0.01;
    vec3 col;
    col.r = TEX2D(u_tex, uv + vec2(ab, 0.0)).r;
    col.g = TEX2D(u_tex, uv).g;
    col.b = TEX2D(u_tex, uv - vec2(ab, 0.0)).b;

    // Chroma bleed toward the left-smeared sample.
    col = mix(col, sample_bleed(uv), clamp(u_bleed, 0.0, 1.0) * 0.6);

    // Saturation grade.
    float l = dot(col, vec3(0.299, 0.587, 0.114));
    col = mix(vec3(l), col, 1.0 + u_sat);

    // Scanlines.
    float s = 0.5 + 0.5 * sin(uv.y * u_res.y * 3.14159);
    col *= mix(1.0, s, u_scan);

    // Tape grain.
    float n = av_hash(uv * u_res.xy + u_time * 60.0) - 0.5;
    col += n * u_noise * 0.3;

    // Vignette.
    vec2 d = uv - 0.5;
    float vig = smoothstep(0.9, 0.3, dot(d, d) * 2.0);
    col *= mix(1.0, vig, u_vignette);

    FRAG_COLOR = vec4(col, 1.0);
}
