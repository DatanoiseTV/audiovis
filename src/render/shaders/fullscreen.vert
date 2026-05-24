// Shared vertex stage for every full-screen pass. Derives a 0..1 UV from the
// oversized clip-space triangle so fragment shaders can sample in UV space.
ATTRIBUTE vec2 a_pos;
VARYING vec2 v_uv;

void main() {
    v_uv = a_pos * 0.5 + 0.5;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
