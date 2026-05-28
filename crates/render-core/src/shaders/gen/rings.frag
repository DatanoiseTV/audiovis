// Concentric rings that ripple outward; bass energy widens the spacing.
void main() {
    vec2 p = av_coord();
    float r = length(p);
    float t = av_t();
    float freq = u_scale * 10.0 + u_audio.x * 12.0;
    float v = 0.5 + 0.5 * sin(r * freq - t * 3.0);
    v = pow(v, mix(1.0, 6.0, u_p1));
    vec3 col = av_hsv(u_hue + r * 0.3, 0.8, v);
    FRAG_COLOR = vec4(col, 1.0);
}
