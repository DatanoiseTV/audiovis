// Displays the live camera frame (uploaded on the u_wave unit). Uses av_coord so
// the layer transform (zoom / rotate / pan) and the Scale knob all work; the
// frame fills the layer at defaults. Camera frames are top-down, so flip v.
void main() {
    vec2 p = av_coord() / max(u_scale, 0.05);
    float aspect = u_res.x / max(u_res.y, 1.0);
    vec2 uv = vec2(p.x / aspect, p.y) * 0.5 + 0.5;
    uv.y = 1.0 - uv.y;
    FRAG_COLOR = TEX2D(u_wave, uv);
}
