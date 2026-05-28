// Domain-warped fBm flow field - smoky, organic, slow.
void main() {
    vec2 p = av_coord() * u_scale * 1.5;
    float t = av_t() * 0.3;
    vec2 q = vec2(av_fbm(p + t), av_fbm(p + vec2(5.2, 1.3) - t));
    vec2 r = vec2(
        av_fbm(p + 4.0 * q + vec2(1.7, 9.2) + u_warp * 3.0),
        av_fbm(p + 4.0 * q + vec2(8.3, 2.8) - t)
    );
    float v = av_fbm(p + 4.0 * r);
    vec3 col = av_hsv(u_hue + v + length(q) * 0.2, 0.7, 0.25 + 0.75 * v);
    FRAG_COLOR = vec4(col, 1.0);
}
