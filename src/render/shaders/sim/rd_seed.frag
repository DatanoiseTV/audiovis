// Seed reaction-diffusion: A = 1 everywhere, B in scattered chunky blobs.
void main() {
    float h = av_hash(floor(v_uv * vec2(48.0, 32.0)));
    float b = step(0.86, h);
    FRAG_COLOR = vec4(1.0, b, 0.0, 1.0);
}
