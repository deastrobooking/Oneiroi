# oneiroi

Four-deck live-performance video mixer with GPU-native HAP playback and
FFmpeg fallback import.

## Status

The display foundation and first HAP credibility slice are proven on the
target OS:

- wgpu + winit + egui render loop with explicit linear/sRGB handling.
- Vidvox's reference HAP decoder behind a bounded safe Rust API.
- BC1, BC3, BC4, BC6H and BC7 plane models, including HAP Q and HAP Q Alpha.
- Direct block-compressed upload and sampling; HAP is never expanded to CPU
  RGBA.
- MOV probing and raw HAP packet reads through libavformat, with exact rational
  timestamps.
- A bounded generation-safe frame scheduler with hold, drop, late, repeat and
  invalidation accounting.
- Four independently assignable decks with non-blocking drag-and-drop movie
  probing and stale-import rejection.
- Import health ratings for HAP, ProRes, DNxHD/DNxHR, H.264, H.265 and other
  FFmpeg-decodable movie formats.

Pixel decode for non-HAP movies, four-way GPU composition, seeking/loop
prefetch, effects and MIDI have not landed yet.

```sh
cargo run          # the app
cargo test         # includes a headless GPU readback test
cargo run -p oneiroi-media --example probe_movie -- footage.mp4
cargo run -p oneiroi-render --example dump_frame > frame.raw \
  && ffmpeg -f rawvideo -pix_fmt rgba -s 512x512 -i frame.raw -y frame.png
```

In the app, click deck A, B, C or D and drag a movie onto the window. Imports
are probed on a background worker and the next deck is selected automatically.

## Layout

| Crate | Owns |
|---|---|
| `oneiroi-core` | Clock, parameters, modulation, scene graph. No GPU, no I/O — testable headless. |
| `oneiroi-hap-sys` | Pinned Vidvox HAP C reference source and raw bindings. |
| `oneiroi-hap` | Bounded safe HAP decode to GPU-native BC planes. |
| `oneiroi-media` | Demux, codec dispatch, frame queues and scheduling. |
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
