// Cheap single-pass bloom: gather a small bright-pass neighbourhood and add it
// back as glow. Prepended with lib.glsl (gives u_res).
uniform sampler2D u_tex;
uniform float u_amount;
uniform float u_threshold;

void main() {
    vec3 base = TEX2D(u_tex, v_uv).rgb;
    vec3 b = vec3(0.0);
    for (int x = -2; x <= 2; x++) {
        for (int y = -2; y <= 2; y++) {
            vec2 o = vec2(float(x), float(y)) * (2.5 / u_res);
            vec3 s = TEX2D(u_tex, v_uv + o).rgb;
            float br = max(s.r, max(s.g, s.b));
            b += s * max(0.0, br - u_threshold);
        }
    }
    b /= 25.0;
    FRAG_COLOR = vec4(base + b * u_amount * 4.0, 1.0);
}
