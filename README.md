# oneiroi

Live-performance video app: real-time clip playback with a GPU effect chain.

## Status

Milestone 1 of 10 — wgpu + winit + egui proven on the target OS. A spinning
triangle driven by a uniform buffer, with an egui overlay whose slider feeds
that uniform. No media, no effects, no MIDI yet.

```sh
cargo run          # the app
cargo test         # includes a headless GPU readback test
cargo run -p oneiroi-render --example dump_frame > frame.raw \
  && ffmpeg -f rawvideo -pix_fmt rgba -s 512x512 -i frame.raw -y frame.png
```

## Layout

| Crate | Owns |
|---|---|
| `oneiroi-core` | Clock, parameters, modulation, scene graph. No GPU, no I/O — testable headless. |
| `oneiroi-media` | Decode: demux, HAP, ffmpeg fallbacks, frame ring buffers. Empty until M2. |
| `oneiroi-render` | wgpu device, surface, render passes. Knows nothing about winit or egui. |
| `oneiroi-io` | MIDI, OSC, audio capture, Ableton Link. Empty until M7. |
| `oneiroi-app` | Windowing, UI, wiring. |

## Decisions already locked in

**wgpu is pinned to 29** because `egui-wgpu` 0.35 depends on 29.0. Bumping wgpu
alone puts two incompatible copies in the tree; bump them together.

**Colour space.** The swapchain is configured in gamma space with the sRGB
variant listed as a view format. Content renders through the `*Srgb` view — so
shaders work in linear and the hardware encodes on write — and the egui overlay
renders through the plain gamma-space view, which is what it wants. There is no
manual `pow(2.2)` anywhere and there should never be one.

**Presentation is Fifo (vsync).** Clip playback gets its own time-based clock
later; frame counters are not a clock when clips run at 24/25/30fps.

## Not decided yet

- Ableton Link is GPLv2+ or a proprietary licence obtained from Ableton. Settle
  this before `oneiroi-io` depends on it.
- Whether ffmpeg is vendored and statically linked, and the LGPL consequences
  for distribution.
