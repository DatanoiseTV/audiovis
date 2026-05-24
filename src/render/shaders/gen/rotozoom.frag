// Rotozoomer: the Amiga-era rotating, zooming textured plane.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float z = 1.0 + 0.5 * sin(t * 0.5);
    p = av_rot(t * 0.3) * p / (z * u_scale);
    vec2 g = floor(p * 8.0);
    float c = mod(g.x + g.y, 2.0);
    float n = av_fbm(p * 2.0 + u_warp * t);
    vec3 col = mix(av_hsv(u_hue, 0.7, 0.9), av_hsv(u_hue + 0.4, 0.8, 0.4), c);
    FRAG_COLOR = vec4(col * (0.7 + 0.3 * n), 1.0);
}
