// A bar spectrum. Reacts to band energy; animates gently with no audio.
void main() {
    vec2 uv = v_uv;
    float bars = 16.0 + floor(u_p1 * 48.0);
    float idx = floor(uv.x * bars);
    float seed = idx / bars;

    float audio = mix(u_audio.x, u_audio.z, seed) * 1.5;
    float h = 0.1 + audio + 0.3 * abs(sin(av_t() * 1.5 + seed * 10.0));
    float bar = step(uv.y, clamp(h, 0.0, 1.0));
    float fx = fract(uv.x * bars);
    float gap = smoothstep(0.0, 0.04, fx) * smoothstep(0.0, 0.04, 1.0 - fx);

    vec3 col = av_hsv(u_hue + seed * 0.5, 0.85, 1.0) * bar * gap;
    FRAG_COLOR = vec4(col, 1.0);
}
