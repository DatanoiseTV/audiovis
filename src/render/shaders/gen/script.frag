// Displays the JS script's 2D pixel buffer (supplied on the u_wave unit). Uses
// av_coord so the layer transform (zoom / rotate / pan) and the Scale knob work;
// the buffer fills the layer at defaults. The buffer is authored top-down (row 0
// at the top), so flip v.
void main() {
    vec2 p = av_coord() / max(u_scale, 0.05);
    float aspect = u_res.x / max(u_res.y, 1.0);
    vec2 uv = vec2(p.x / aspect, p.y) * 0.5 + 0.5;
    uv.y = 1.0 - uv.y;
    FRAG_COLOR = TEX2D(u_wave, uv);
}
