// Infinite perspective grid floor + ceiling racing toward a horizon.
void main() {
    vec2 uv = v_uv * 2.0 - 1.0;
    float t = av_t();
    float y = uv.y;
    if (abs(y) < 0.02) y = 0.02 * sign(y + 0.0001);
    float z = 1.0 / abs(y);            // perspective depth
    vec2 g = vec2(uv.x * z, z + t * 2.0);
    vec2 f = abs(fract(g * u_scale) - 0.5);
    float line = step(0.46, max(f.x, f.y));
    float fog = clamp(1.0 - abs(y), 0.0, 1.0);
    vec3 col = av_hsv(u_hue + z * 0.02, 0.7, line * fog);
    FRAG_COLOR = vec4(col, 1.0);
}
