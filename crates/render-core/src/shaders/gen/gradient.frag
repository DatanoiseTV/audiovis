// Mesh gradient: a few soft colour blobs drift and blend into a smooth gradient
// field - the "premium app background" look. High band lifts the brightness.
void main() {
    vec2 uv = av_coord();
    float t = av_t() * 0.4;
    vec3 col = vec3(0.0);
    float wsum = 0.0;
    for (int i = 0; i < 4; i++) {
        float fi = float(i);
        vec2 c = 0.85 * vec2(sin(t + fi * 1.7), cos(t * 0.8 + fi * 2.3));
        float d = length(uv - c);
        float w = 1.0 / (0.12 + d * d * (1.4 + u_scale));
        col += av_hsv(u_hue + fi * 0.18 + u_audio.x * 0.1, 0.62, 1.0) * w;
        wsum += w;
    }
    col /= max(wsum, 0.001);
    col *= 0.7 + 0.6 * u_audio.z;
    FRAG_COLOR = vec4(col, 1.0);
}
