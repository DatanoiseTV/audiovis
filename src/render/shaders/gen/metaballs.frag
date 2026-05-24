// Six orbiting metaballs, threshold gives the gooey lava-lamp look.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float m = 0.0;
    for (int i = 0; i < 6; i++) {
        float fi = float(i);
        vec2 c = 0.7 * vec2(sin(t * 0.7 + fi * 1.3), cos(t * 0.6 + fi * 2.1));
        float rad = 0.12 * (1.0 + u_p1) * (1.0 + u_audio.y);
        m += rad / (length(p - c) + 0.001);
    }
    float v = smoothstep(1.5, 4.0, m * u_scale);
    vec3 col = av_hsv(u_hue + m * 0.05, 0.7, v);
    FRAG_COLOR = vec4(col, 1.0);
}
