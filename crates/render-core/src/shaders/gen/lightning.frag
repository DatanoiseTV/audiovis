// Branching electric bolts, flickering with the highs.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float bolt = 0.0;
    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        float x = 0.3 * (fi - 1.0) + 0.35 * (av_fbm(vec2(p.y * 3.0 + fi * 10.0, t * 3.0)) - 0.5) * (1.0 + u_warp * 3.0);
        bolt += 0.012 / (abs(p.x - x) + 0.012);
    }
    bolt *= 0.5 + 0.5 * sin(t * 10.0 + p.y * 4.0);
    vec3 col = av_hsv(u_hue + 0.6, 0.4, 1.0) * bolt * (0.5 + u_audio.z * 2.0);
    FRAG_COLOR = vec4(col, 1.0);
}
