// Bokeh: soft out-of-focus light orbs drifting upward, additive. A dreamy,
// modern depth-of-field look. Low-band energy makes the orbs swell.
void main() {
    vec2 uv = av_coord();
    float t = av_t() * 0.2;
    vec3 col = vec3(0.0);
    for (int i = 0; i < 14; i++) {
        float seed = float(i) * 1.37;
        vec2 p = vec2(
            (av_hash(vec2(seed, 1.0)) * 2.0 - 1.0) * 1.6,
            fract(av_hash(vec2(seed, 2.0)) + t * (0.3 + 0.5 * av_hash(vec2(seed, 3.0)))) * 2.4 - 1.2
        );
        float r = 0.07 + 0.2 * av_hash(vec2(seed, 4.0));
        float d = length(uv - p);
        float orb = smoothstep(r, r * 0.15, d) * (0.35 + u_audio.x * 0.9);
        col += av_hsv(u_hue + float(i) * 0.05, 0.45, 1.0) * orb;
    }
    FRAG_COLOR = vec4(col, 1.0);
}
