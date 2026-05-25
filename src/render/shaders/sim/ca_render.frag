// Colourise the excitable medium: excited cells glow bright, fading through the
// refractory tail, resting cells dark. Hue tracks the phase so waves are tinted.
uniform sampler2D u_state;
uniform vec2 u_texel;

void main() {
    float p = TEX2D(u_state, v_uv).r; // state / N, in 0..1
    float bright = p < 0.02 ? 0.04 : (1.0 - p) * 0.9 + 0.12;
    vec3 col = av_hsv(u_hue + p * 0.8 + 0.5, 0.85, bright);
    FRAG_COLOR = vec4(col, 1.0);
}
