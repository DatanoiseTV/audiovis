// Placeholder for the "isf" generator slot. The compositor renders the selected
// ISF shader directly (see render/isf.rs); this only exists so the generator
// bank compiles a program for the slot. Shown (black) when no ISF is selected.
void main() {
    FRAG_COLOR = vec4(0.0, 0.0, 0.0, 1.0);
}
