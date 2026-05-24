// Drifting fBm clouds over a sky gradient.
void main() {
    vec2 p = av_coord() * u_scale + vec2(av_t() * 0.1, 0.0);
    float n = av_fbm(p * 2.0 + av_fbm(p + av_t() * 0.05) * u_warp * 3.0);
    n = smoothstep(0.3, 0.8, n);
    vec3 sky = av_hsv(u_hue + 0.55, 0.6, 0.3);
    vec3 cloud = av_hsv(u_hue, 0.1, 1.0);
    FRAG_COLOR = vec4(mix(sky, cloud, n), 1.0);
}
