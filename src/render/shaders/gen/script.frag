// Displays the JS script's 2D pixel buffer (supplied on u_wave). The buffer is
// authored top-down (row 0 at the top), so flip v. Nearest filtering on the
// texture keeps it crisp and pixel-art-like.
void main() {
    vec2 c = vec2(v_uv.x, 1.0 - v_uv.y);
    FRAG_COLOR = TEX2D(u_wave, c);
}
