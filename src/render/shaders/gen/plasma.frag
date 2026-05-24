// Classic sine-interference plasma, warped by fBm. The all-rounder.
void main() {
    vec2 p = av_coord() * u_scale * 3.0;
    float t = av_t();
    float v = sin(p.x + t) + sin(p.y * 1.3 + t * 0.7) + sin((p.x + p.y) * 0.7 + t * 1.3);
    v += 1.5 * sin(length(p) * 2.0 - t * 2.0);
    v += u_warp * 4.0 * av_fbm(p * 0.5 + t * 0.1);
    vec3 col = av_hsv(u_hue + v * 0.08 + 0.5, 0.85, 0.55 + 0.45 * sin(v));
    FRAG_COLOR = vec4(col, 1.0);
}
