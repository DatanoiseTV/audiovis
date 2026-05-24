// Checkerboard test pattern dragged through a noise warp.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    p += u_warp * 1.5 * vec2(av_fbm(p * 2.0 + t), av_fbm(p * 2.0 - t));
    vec2 g = floor(p * u_scale * 8.0);
    float c = mod(g.x + g.y, 2.0);
    vec3 col = mix(av_hsv(u_hue, 0.8, 0.9), av_hsv(u_hue + 0.5, 0.8, 0.5), c);
    FRAG_COLOR = vec4(col, 1.0);
}
