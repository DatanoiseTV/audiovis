// Soft focus: a cheap 9-tap gaussian-ish blur for a dreamy, modern bloom-bed.
// (v_uv and u_res come from the prepended lib.)
uniform sampler2D u_tex;
uniform float u_amount; // blur radius

void main() {
    vec2 px = (1.0 / u_res) * (1.0 + u_amount * 9.0);
    vec3 c = TEX2D(u_tex, v_uv).rgb * 0.25;
    c += TEX2D(u_tex, v_uv + vec2(px.x, 0.0)).rgb * 0.125;
    c += TEX2D(u_tex, v_uv - vec2(px.x, 0.0)).rgb * 0.125;
    c += TEX2D(u_tex, v_uv + vec2(0.0, px.y)).rgb * 0.125;
    c += TEX2D(u_tex, v_uv - vec2(0.0, px.y)).rgb * 0.125;
    c += TEX2D(u_tex, v_uv + px).rgb * 0.0625;
    c += TEX2D(u_tex, v_uv - px).rgb * 0.0625;
    c += TEX2D(u_tex, v_uv + vec2(px.x, -px.y)).rgb * 0.0625;
    c += TEX2D(u_tex, v_uv + vec2(-px.x, px.y)).rgb * 0.0625;
    FRAG_COLOR = vec4(c, 1.0);
}
