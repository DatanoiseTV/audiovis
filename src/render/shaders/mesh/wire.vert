// Wireframe mesh vertex stage: rotate the model (yaw/pitch), apply a cheap
// perspective divide and aspect-correct. Positions are pre-normalised to a unit
// sphere by the loader. v_fog fades far edges for depth cueing.
ATTRIBUTE vec3 a_pos;
VARYING float v_fog;

uniform vec2 u_rot;   // x = yaw, y = pitch (radians)
uniform vec2 u_res;   // viewport size, for aspect

void main() {
    float cy = cos(u_rot.x), sy = sin(u_rot.x);
    vec3 p = vec3(a_pos.x * cy - a_pos.z * sy, a_pos.y, a_pos.x * sy + a_pos.z * cy);
    float cx = cos(u_rot.y), sx = sin(u_rot.y);
    p = vec3(p.x, p.y * cx - p.z * sx, p.y * sx + p.z * cx);

    float z = p.z + 3.0;            // push away from the camera
    float f = 2.2 / max(z, 0.1);    // perspective divide
    vec2 sp = p.xy * f;
    sp.x *= u_res.y / max(u_res.x, 1.0); // keep proportions on wide outputs
    v_fog = clamp((z - 1.8) * 0.5, 0.0, 1.0);
    gl_Position = vec4(sp, 0.0, 1.0);
}
