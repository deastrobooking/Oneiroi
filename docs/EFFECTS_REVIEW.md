# Effect and custom shader review — 2026-09-25

The current effect runtime is suitable for expanding the library: typed package
manifests, bounded one/two-pass master processing, one-pass deck processing,
background compilation, last-known-good reload and generated parameter controls
already work. This pass adds visual variety without changing the runtime ABI.

## Findings addressed

- Recursive 2D and Hyper Recursion could repeatedly square escaped coordinates
  until they overflowed, producing invalid texture coordinates. Their orbits
  are now clamped after each iteration. Fractal Volume uses the same bound for
  its growing field coordinates. Iteration/sample counts remain fixed.
- New shaders keep transparent inputs from contributing hidden RGB to highlight
  extraction or color separation. CRT returns zero RGB outside its curved
  image, and Thermal preserves the original alpha.
- The flare extraction pass prefilters across the gather interval so sparse
  highlights create smoother streaks rather than separated copies.
- New looks are complete presets, specifying every package parameter. They do
  not depend on whichever values happened to be selected previously.

## Added packages

| Effect | Placement | Presets |
|---|---|---|
| Analog CRT | Deck and master | Broadcast, Arcade, Worn tape |
| Thermal Contours | Deck and master | Iron heat, Aurora map, Topography |
| Gravitational Lens | Deck and master | Singularity, Liquid orbit, Repulsor |
| Anamorphic Flare | Master, two passes | Cinema blue, Golden hour, Laser streaks |
| Kaleidoscope | Deck and master | Sixfold, Stained glass, Spiral bloom |
| Mirror Symmetry | Deck and master | Bilateral, Four-way, Diagonal |
| Mirror Mosaic | Deck and master | Hall of mirrors, Diamond glass, Moving grid |

Each integrates with existing dry/wet, bypass, named parameter persistence,
MIDI Learn and modulation. Refresh the package registry or restart the app to
discover newly installed folders. See [package authoring](EFFECT_PACKAGES.md)
for controls and the preview command.

## Validation

- Workspace: 291 tests passed, zero failures, one separately opt-in decoder soak.
- Strict Clippy for all targets/features, formatting and release build passed.
- GPU readback covers the new deck presets, transparent input, bypass and dry
  identity; master tests exercise the new one/two-pass packages and presets.
- Mirror GPU readback verifies reflected source pixels, both axes, source-side
  selection and fourfold radial symmetry. Master readback verifies constant-field
  coverage at all presets and parameter extremes. All nine looks were previewed.
- Existing recursive packages are exercised at extreme settings with opaque
  input to check coverage after escaping polynomial iterations.
- The `effect_preview` example renders a repeatable synthetic chart through
  the actual master runtime. Its output was visually inspected.

Short synthetic benchmark on Apple M3 Pro / Metal: four 1920×1080 HAP BC1
sources, neutral built-ins, no master effects, one package per deck, 30 warmup
frames followed by 120 measured frames, one run per package:

| Package | Sustained ms/frame | Synchronous p95 ms/frame |
|---|---:|---:|
| Analog CRT | 3.86 | 6.02 |
| Thermal Contours | 3.69 | 7.35 |
| Gravitational Lens | 2.84 | 5.83 |

These measure the benchmark pipeline rather than isolated effect cost. They
are development samples, not a show-machine soak or UHD certification. Flare
was GPU-tested and visually checked but was not included in this deck-only
benchmark. It uses five extraction samples plus 51 gather samples per pixel.

## Next work

1. Run the new looks with four real sources, the intended master chain and
   output display on the show machine; record sustained and tail frame times.
2. Add parameter/preset thumbnails to make the larger catalog easier to browse.
3. Introduce shared shader helpers only with ABI and alpha regression coverage;
   duplicated globals/sampling conventions are currently easy to drift.
4. Keep multi-frame slit-scan, optical flow and temporal deck packages separate
   from this stateless expansion; they need additional resource contracts.
