// Aurora: drifting curtains of light from layered fBm. Smooth gradients, no
// hard edges - a clean, modern look. Mid-band energy brightens it.
void main() {
    vec2 uv = av_coord();
    float t = av_t() * 0.3;
    float warp = av_fbm(vec2(uv.x * 1.5 + t, uv.y * 0.6 - t * 0.5)) * (0.6 + u_warp);
    float x = uv.x * 1.2 + warp;
    float band = 0.0;
    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        float c = sin(x * (1.5 + fi * 0.7) + t * (1.0 + fi * 0.3) + fi * 2.0);
        float curtain = smoothstep(0.0, 0.85, c) * exp(-abs(uv.y + 0.2 - fi * 0.28) * (1.4 + u_p2 * 3.0));
        band += curtain;
    }
    band *= 1.0 + u_audio.y * 1.6;
    vec3 col = av_hsv(u_hue + uv.x * 0.08 + band * 0.15, 0.55, 1.0) * band;
    col += av_hsv(u_hue + 0.5, 0.35, 1.0) * band * band * 0.3; // brighter core
    FRAG_COLOR = vec4(col, 1.0);
}
