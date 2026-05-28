// Seed the cyclic CA with random phases - spirals nucleate out of the noise.
void main() {
    FRAG_COLOR = vec4(av_hash(v_uv * u_res), 0.0, 0.0, 1.0);
}
