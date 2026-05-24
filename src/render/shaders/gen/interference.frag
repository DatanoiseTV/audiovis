// Ripple interference from several moving point sources.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float v = 0.0;
    for (int i = 0; i < 5; i++) {
        float fi = float(i);
        vec2 src = 0.7 * vec2(sin(t * 0.5 + fi * 1.3), cos(t * 0.4 + fi * 2.1));
        v += sin(length(p - src) * (16.0 + 8.0 * u_scale) - t * 4.0);
    }
    v /= 5.0;
    vec3 col = av_hsv(u_hue + v * 0.5, 0.7, 0.5 + 0.5 * v);
    FRAG_COLOR = vec4(col, 1.0);
}
