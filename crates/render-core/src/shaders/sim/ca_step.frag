// Greenberg-Hastings excitable medium - the model behind spiral/target waves
// in chemical reactions (Belousov-Zhabotinsky). States: 0 = resting,
// 1 = excited, 2..N-1 = refractory countdown back to rest. A resting cell fires
// when enough neighbours are excited; an excited/refractory cell advances each
// step. From noise this self-organises into rotating spirals fast.
// Param 1 = refractory length, Param 2 / mids = excitation threshold.
uniform sampler2D u_state;
uniform vec2 u_texel;

float ph(vec2 uv, float n) {
    return mod(floor(TEX2D(u_state, uv).r * n + 0.5), n);
}

void main() {
    float n = 4.0 + floor(u_p1 * 14.0);
    float me = ph(v_uv, n);
    float result;
    if (me < 0.5) {
        // Resting: fire if enough neighbours are excited (state == 1).
        float exc = 0.0;
        for (int x = -1; x <= 1; x++) {
            for (int y = -1; y <= 1; y++) {
                if (abs(ph(v_uv + vec2(float(x), float(y)) * u_texel, n) - 1.0) < 0.5) exc += 1.0;
            }
        }
        // Threshold 1 (default) self-sustains spirals; higher freezes them.
        float thresh = 1.0 + floor(u_p2 * 1.5) + floor(u_audio.y * 2.0);
        result = exc >= thresh ? 1.0 : 0.0;
        // Rare spontaneous sparks keep the medium alive and continually
        // nucleate new spiral cores; the bass opens the spark rate.
        float spark = av_hash(v_uv * u_res * 1.7 + u_time * 13.0);
        if (result < 0.5 && spark > 0.9994 - u_audio.x * 0.004) result = 1.0;
    } else {
        // Excited / refractory: advance toward rest.
        result = mod(me + 1.0, n);
    }
    FRAG_COLOR = vec4(result / n, 0.0, 0.0, 1.0);
}
