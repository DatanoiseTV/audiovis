// Wireframe mesh fragment stage: a hue-tinted line, dimmed with distance.
VARYING float v_fog;
uniform float u_hue;
uniform float u_audio; // low-band energy, brightens the lines on hits

vec3 hsv(float h, float s, float v) {
    vec3 k = mod(h * 6.0 + vec3(5.0, 3.0, 1.0), 6.0);
    k = clamp(min(k, 4.0 - k), 0.0, 1.0);
    return v * mix(vec3(1.0), k, s);
}

void main() {
    vec3 col = hsv(u_hue, 0.65, 1.0) * (0.85 + u_audio * 0.6);
    col *= 1.0 - v_fog * 0.6;
    FRAG_COLOR = vec4(col, 1.0);
}
