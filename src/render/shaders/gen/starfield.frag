// Three parallax layers of drifting stars.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    vec3 col = vec3(0.0);
    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        float sp = (fi + 1.0) * u_scale;
        vec2 q = p * sp * 4.0 + vec2(0.0, t * (fi + 1.0) * u_p1 * 4.0);
        vec2 cell = floor(q);
        vec2 f = fract(q) - 0.5;
        float h = av_hash(cell + fi * 13.0);
        float star = smoothstep(0.09, 0.0, length(f)) * step(0.92, h);
        col += vec3(star) * (0.4 + 0.2 * fi);
    }
    col *= av_hsv(u_hue, 0.3, 1.0);
    FRAG_COLOR = vec4(col, 1.0);
}
