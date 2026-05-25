// Present the dye field directly (it already carries colour).
uniform sampler2D u_state;
uniform vec2 u_texel;

void main() {
    FRAG_COLOR = vec4(TEX2D(u_state, v_uv).rgb, 1.0);
}
