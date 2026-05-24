// Truchet tiles: randomly flipped quarter-arcs forming endless mazes.
void main() {
    vec2 p = av_coord() * u_scale * 3.0 + av_t() * 0.2;
    vec2 g = floor(p);
    vec2 f = fract(p);
    float h = av_hash(g);
    if (h < 0.5) f.x = 1.0 - f.x; // flip half the tiles
    float d = min(length(f - vec2(0.0, 1.0)), length(f - vec2(1.0, 0.0)));
    float line = smoothstep(0.09, 0.0, abs(d - 0.5));
    vec3 col = av_hsv(u_hue + h * 0.3, 0.7, line);
    FRAG_COLOR = vec4(col, 1.0);
}
