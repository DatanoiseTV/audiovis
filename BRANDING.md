# Branding & UI style

The web control surface should feel like it belongs next to a video mixer and a
stack of CRTs in a dark club: an analog-video instrument, not a settings page.
The look is for a live video instrument - abstract, analog, demoscene, techno.

## Mood

- Dark room, glowing phosphor. High contrast, low overall brightness.
- Reads instantly in the dark and at a glance while performing.
- Hints of chroma separation (the red/blue split of a misaligned signal) and
  scanline texture, used sparingly so the controls stay legible.

## Palette

Base
- `--bg`        `#0a0a0f`  near-black, slight blue
- `--panel`     `#12121a`  raised surfaces
- `--line`      `#262633`  hairlines / borders
- `--text`      `#d7d7e0`  primary text
- `--text-dim`  `#7a7a8c`  secondary text

Signal accents (the chroma split)
- `--cyan`      `#35e0d8`  primary accent, active controls
- `--magenta`   `#ff3ea5`  secondary accent, modulation / live values
- `--amber`     `#ffb347`  warnings, clip/peak

Queer accent ramp (use for spectra, meters, gradients - not chrome)
- magenta -> orange -> yellow -> green -> blue, drawn from the rainbow but
  desaturated toward the CRT phosphor feel rather than flat flag colors.

## Type

- Monospace for values, addresses and labels (it is an instrument). System mono
  stack: `ui-monospace, "SF Mono", "DejaVu Sans Mono", monospace`.
- Tight, uppercase, slightly tracked-out section headers.

## Components

- Knobs and faders over numeric inputs; show the live value next to each.
- Meters glow and bloom slightly on peaks.
- A "learn" affordance on every control: arm it, then move a MIDI/OSC control to
  bind it.
- No emoji. No rounded "friendly" UI. Crisp, instrument-grade.
