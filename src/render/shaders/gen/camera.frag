// Displays the live camera frame (uploaded on the u_wave unit). Camera frames
// are top-down, so flip v. Shows black until a frame arrives.
void main() {
    vec2 c = vec2(v_uv.x, 1.0 - v_uv.y);
    FRAG_COLOR = TEX2D(u_wave, c);
}
