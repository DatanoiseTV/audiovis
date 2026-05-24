// Video feedback: blend the current frame with the previous output, sampled
// with a slow zoom + rotate, for infinite-tunnel trails (the camera-into-its-
// own-monitor demoscene/VJ classic). Prepended with lib.glsl (av_rot, TEX2D).
uniform sampler2D u_tex;      // current frame
uniform sampler2D u_history;  // previous output
uniform float u_amount;       // trail persistence 0..1
uniform float u_zoom;         // per-frame zoom of the fed-back image
uniform float u_rotate;       // per-frame rotation

void main() {
    vec2 c = v_uv - 0.5;
    c = av_rot(u_rotate * 0.04) * c * (1.0 - u_zoom * 0.04);
    vec3 hist = TEX2D(u_history, c + 0.5).rgb;
    vec3 cur = TEX2D(u_tex, v_uv).rgb;
    // `max` keeps bright sources punchy while the decayed history trails.
    FRAG_COLOR = vec4(max(cur, hist * clamp(u_amount, 0.0, 0.995)), 1.0);
}
