// Sunflower phyllotaxis: seeds placed on the golden angle, slowly rotating.
void main() {
    vec2 p = av_coord();
    float t = av_t();
    float d = 1e9;
    const float n = 64.0;
    for (int i = 0; i < 64; i++) {
        float fi = float(i);
        float ang = fi * 2.39996323 + t * 0.3;
        float rad = sqrt(fi / n) * 1.2 * u_scale;
        vec2 q = rad * vec2(cos(ang), sin(ang));
        d = min(d, length(p - q));
    }
    float seed = smoothstep(0.045, 0.0, d);
    float glow = 0.015 / (d + 0.02);
    vec3 col = av_hsv(u_hue + d, 0.7, seed + glow * 0.4);
    FRAG_COLOR = vec4(col, 1.0);
}
