// Hexagonal grid that pulses outward from the centre with the bass.
void main() {
    vec2 p = av_coord() * u_scale * 4.0;
    vec2 r = vec2(1.0, 1.7320508);
    vec2 hh = r * 0.5;
    vec2 a = mod(p, r) - hh;
    vec2 b = mod(p - hh, r) - hh;
    vec2 gv = dot(a, a) < dot(b, b) ? a : b;
    float hexd = max(abs(gv.x) * 0.8660254 + abs(gv.y) * 0.5, abs(gv.y));
    vec2 id = p - gv;
    float pulse = 0.5 + 0.5 * sin(av_t() * 2.0 + length(id) * 0.5 - u_audio.x * 5.0);
    float edge = smoothstep(0.5, 0.44, hexd);
    vec3 col = av_hsv(u_hue + length(id) * 0.04, 0.7, edge * pulse);
    FRAG_COLOR = vec4(col, 1.0);
}
