// Modern chromatic aberration: a clean radial RGB split that grows toward the
// edges, with an optional soft vignette. Subtle and contemporary (not glitchy).
// (v_uv comes from the prepended lib.)
uniform sampler2D u_tex;
uniform float u_amount;   // aberration strength
uniform float u_vignette; // edge darkening

void main() {
    vec2 d = v_uv - 0.5;
    float r2 = dot(d, d);
    vec2 off = d * r2 * u_amount * 0.7;
    vec3 col = vec3(
        TEX2D(u_tex, v_uv + off).r,
        TEX2D(u_tex, v_uv).g,
        TEX2D(u_tex, v_uv - off).b
    );
    col *= 1.0 - u_vignette * r2 * 1.4;
    FRAG_COLOR = vec4(col, 1.0);
}
