// Spiralling wormhole: angular twist that tightens toward the centre.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float a = atan(p.y, p.x);
    float r = length(p);
    float twist = a + 2.0 / (r + 0.1) + t;
    float depth = log(r + 0.05) * 3.0 - t * 2.0;
    float v = 0.5 + 0.5 * sin(twist * (3.0 + floor(u_p1 * 8.0))) * sin(depth * PI * u_scale);
    float fog = clamp(r * 1.6, 0.0, 1.0);
    vec3 col = av_hsv(u_hue + depth * 0.05, 0.7, v * fog);
    FRAG_COLOR = vec4(col, 1.0);
}
