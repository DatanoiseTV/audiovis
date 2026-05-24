# audiovis

A single-binary live audio-reactive VJ visualizer with a VHS / retro / analog-video
look. It generates visuals algorithmically (no pre-rendered loops required),
reacts to live audio, and is driven in performance over **MIDI**, **OSC** and a
**web control surface**.

It is built to run both on a capable desktop (windowed output on macOS and Linux)
and on very small hardware: single-core ~1 GHz ARM boards such as the Raspberry Pi
Zero or NTC C.H.I.P., rendering straight to the framebuffer with no X11 or Wayland.

## Design goals

- **One static binary.** Drop it on the target and run it; assets are embedded.
- **Runs on GLES2.** The render core targets OpenGL ES 2.0 so it works on the
  weak GPUs found on those small boards (VideoCore IV, Mali-400) and on desktop
  GL alike. Shaders are written to the GLSL ES 1.00 / GLSL 1.20 common subset.
- **Many generators.** A library of algorithmic visual sources (feedback,
  plasma, reaction-diffusion, scopes, moire, flow fields, and more), layered and
  composited, then run through an analog/VHS post chain.
- **Everything is mappable.** Any MIDI control, OSC address or web widget can be
  bound to any parameter of any active generator or effect, with MIDI/OSC learn.

## Output backends

| Platform        | Backend | Notes                                              |
|-----------------|---------|----------------------------------------------------|
| macOS           | window  | winit + glutin, desktop GL                         |
| Linux (desktop) | window  | winit + glutin                                     |
| Linux (headless)| drm     | DRM/KMS + GBM + EGL straight to the framebuffer    |

`--backend auto` selects a sensible default per platform.

## Control

- **MIDI** in (notes, CC, clock) via the system MIDI stack.
- **OSC** over UDP.
- **Web UI** served by the binary itself: a websocket pub/sub channel carrying
  protobuf-encoded messages, with a control surface in the browser.

## Building

```sh
cargo build --release
```

The optional camera / video-file input layer is behind a feature flag:

```sh
cargo build --release --features camera
```

## Running

```sh
audiovis --backend auto --width 1280 --height 720 --fps 60
```

On a small board you will typically lower the cost:

```sh
audiovis --backend drm --width 1280 --height 720 --render-scale 0.5 --fps 30
```

Run `audiovis --help` for the full list of options. Every option also has an
`AV_*` environment-variable equivalent.

## Status

Early development. See the milestones being built out in the source tree:
core engine and control bus, GLES2 render pipeline, generator library, analog
post chain, audio analysis, MIDI/OSC, web UI, and the Linux DRM backend.

## License

MIT.
