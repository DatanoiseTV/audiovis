// XY oscilloscope: a glowing Lissajous curve traced by sampled points.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float a = 2.0 + floor(u_p1 * 6.0);
    float b = 3.0 + floor(u_p2 * 6.0);
    float d = 1e9;
    for (int i = 0; i < 48; i++) {
        float u = float(i) / 48.0 * PI;
        vec2 q = 0.8 * vec2(sin(a * u + t), sin(b * u + t * 0.5));
        d = min(d, length(p - q));
    }
    float line = smoothstep(0.025, 0.0, d);
    float glow = 0.02 / (d + 0.02);
    vec3 col = av_hsv(u_hue, 0.7, 1.0) * (line + glow * (0.4 + u_audio.z));
    FRAG_COLOR = vec4(col, 1.0);
}
