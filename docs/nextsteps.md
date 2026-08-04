Updated todo list

Searched for files matching `**/crates/oneiroi-media/src/decode_ffmpeg.rs`, 1 match

Read [](file:///Users/home/Desktop/ECOSYSTEM/Oneiroi/crates/oneiroi-media/src/decode_ffmpeg.rs#1-1), lines 1 to 80

Searched for text `ffmpeg::format::input` (`**/crates/oneiroi-media/src/decode_ffmpeg.rs`), 2 results

Read [](file:///Users/home/Desktop/ECOSYSTEM/Oneiroi/crates/oneiroi-media/src/decode_ffmpeg.rs#130-130), lines 130 to 220

Searched for files matching `**/crates/oneiroi-app/src/media.rs`, 1 match

Option 2 (**SRT / RTSP Live Feed via FFmpeg**) is a great fit for Oneiroi because **`oneiroi-media` already uses `ffmpeg_next`** for live camera capture and hardware video decoding. 

Adding network live streams requires **zero new heavy dependencies** or external server binaries — FFmpeg already natively handles SRT, RTSP, RTMP, and HTTP streams!

---

### How the Phone Stream Workflow Works

1. **On the Phone (iOS / Android):**
   * Install a free, high-performance camera streaming app like **Larix Broadcaster** or **OBS Camera**.
   * Set output to **SRT** or **RTSP**.
   * Enter the Oneiroi host IP (e.g. `srt://192.168.1.120:9000?mode=listener` or `rtsp://192.168.1.120:8554/live`).
   * Tap record/stream.

2. **In Oneiroi:**
   * In the deck slot media chooser, select **"Connect Network Stream"**.
   * Enter the stream listener URL or port.
   * Oneiroi ingests, decodes, and routes the feed directly into the deck mixer like any local clip or webcam.

---

### Implementation Blueprint for Your Agent

Here is the exact technical plan your agent can follow to add SRT/RTSP stream support to `oneiroi-media` and `oneiroi-app`:

#### 1. Define Network Stream Schemes & Config in capture.rs
Add stream URL descriptors alongside existing `camera://` logic:

```rust
pub const SRT_SCHEME: &str = "srt://";
pub const RTSP_SCHEME: &str = "rtsp://";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkStreamConfig {
    pub url: String,
    pub max_latency_ms: u32, // e.g., 100ms for SRT jitter buffer
}
```

#### 2. Add `open_network_stream` in decode_ffmpeg.rs
Extend `FfmpegVideoDecoder` with a constructor that passes real-time low-latency dictionaries to FFmpeg:

```rust
pub fn open_network_stream(config: &NetworkStreamConfig) -> Result<Self, FfmpegDecodeError> {
    ffmpeg::init().map_err(FfmpegDecodeError::Initialize)?;
    
    let mut options = ffmpeg::Dictionary::new();
    // Low-latency live stream flags
    options.set("fflags", "nobuffer");
    options.set("flags", "low_delay");
    options.set("probesize", "32000");         // Don't wait for large probe buffer
    options.set("analyzeduration", "0");       // Analyze immediately
    options.set("max_delay", "0");
    
    // SRT-specific options (for handling congested Wi-Fi)
    if config.url.starts_with("srt://") {
        options.set("rcvbuf", "16777216");     // 16MB socket buffer
        options.set("latency", &config.max_latency_ms.to_string());
        options.set("overrun_nonfatal", "1");  // Prevent dropped packets from crashing decoder
    }
    
    // RTSP options
    if config.url.starts_with("rtsp://") {
        options.set("rtsp_transport", "tcp");  // TCP transport prevents tearing on packet drop
    }

    let context = ffmpeg::format::open_with(&config.url, &ffmpeg::format::Input::default(), options)
        .map_err(|source| FfmpegDecodeError::Open {
            path: PathBuf::from(&config.url),
            source,
        })?;

    let ffmpeg::format::Context::Input(input) = context else {
        return Err(FfmpegDecodeError::NoVideoStream);
    };

    Self::from_input(
        input,
        false, // allow_hap
        true,  // allow_missing_timestamp
        60,    // fallback_fps
        true,  // live = true (uses non-blocking EAGAIN retry logic)
        FrameBufferPool::new(8),
    )
}
```

#### 3. Wire into the Media Worker in worker.rs
Ensure the media decoding loop marks network streams with `live = true`. When packets are delayed due to Wi-Fi jitter, the existing `is_temporary_live_read_error` handler will gracefully repeat the last valid frame without dropping playback or freezing the UI thread.

#### 4. UI Stream Input Panel in media.rs
In the deck source selection modal:
* Add a **"Stream (SRT / RTSP)"** tab.
* Display a text field for entering `srt://...` or `rtsp://...`.
* Show a helpful IP hint (e.g. `"Listen on srt://0.0.0.0:9000 — set Larix app target to your IP"`).

Created 3 todos


Updated todo list

Read [](file:///Users/home/Desktop/ECOSYSTEM/Oneiroi/effects/chromatic-split/effect.json)

Here is a curated list of high-impact visual effects tailored specifically for Oneiroi's architecture (supporting 1-pass or 2-pass fragment shaders, temporal history buffers, and modulation routing).

---

### 1. Optical & Distortion Effects

#### **Slit-Scan Time Displacement (`slit-scan`)**
* **Concept:** Slices the video into temporal strips based on position or luminosity, creating surreal bending, liquid delay, and rolling shutter distortions.
* **Technique:** Uses binding 5 (`custom_history_texture`) and multi-frame blending or offset mapping.
* **Parameters:**
  * `mode` (Choice: Horizontal, Vertical, Radial, Luma-Driven)
  * `speed` (Scanline propagation rate)
  * `depth` (Temporal offset scale)
  * `feather` (Edge blending between temporal slices)
* **Audio Sync:** Map transient/beat phase to `speed` for rhythmic time distortion pulses.

#### **Anamorphic Streak & Flare (`anamorphic-flare`)**
* **Concept:** Replicates high-end cinema anamorphic lens flares, streak highlights, and optical bloom across bright regions.
* **Technique:** **2-pass effect**.
  * *Pass 0 (`fs_extract`):* Isolates bright pixels using thresholding and applies directional horizontal scaling.
  * *Pass 1 (`fs_combine`):* Performs a 1D horizontal blur ping-pong and overlays chromatic tinting over `original_texture`.
* **Parameters:**
  * `threshold` (Luminance cutoff)
  * `streak_length` (Horizontal bloom spread)
  * `chroma_shift` (Color dispersion on streaks)
  * `tint` (Flare hue angle)

#### **Vector Gravitational Lens (`graviton-warp`)**
* **Concept:** Creates spacetime gravitational lensing distortions that warp and twist input video around movable singularity points.
* **Technique:** 1-pass polar UV coordinate displacement calculated using inverse-square gravitational formulas and angular vortex rotation.
* **Parameters:**
  * `mass` (Lensing warp intensity)
  * `radius` (Event horizon size)
  * `vortex_spin` (Rotational distortion)
  * `center_x`, `center_y` (Singularity focal coordinates)

---

### 2. Retro, Cathode & Glitch FX

#### **Analog Cathode VHS & CRT (`crt-vhs-decay`)**
* **Concept:** Authentic analog TV artifacts including shadow mask phosphor grid, scanlines, luma ringing, tape jitter, and magnetic distortion.
* **Technique:** 1-pass UV warping (barrel distortion) + procedural scanline overlay + NTSC color subcarrier luma/chroma crosstalk simulation.
* **Parameters:**
  * `barrel_distort` (Curved CRT screen geometry)
  * `scanline_opacity` (Line density and visibility)
  * `tape_jitter` (Horizontal sync tracking error)
  * `chroma_bleed` (Color phase smear)

#### **Data-Moshing Simulation (`datamosh-glitch`)**
* **Concept:** Emulates video compression artifacting where optical flow motions smear pixels across frames without keyframe resets.
* **Technique:** Uses `custom_history_texture`. Samples previous frame pixels offset by the local brightness gradient or pseudo-optical flow vector.
* **Parameters:**
  * `flow_speed` (Pixel displacement rate)
  * `decay` (Fade rate of mosh trails)
  * `block_size` (Macroblock quantisation grid)
  * `decay_hold` (Holds frame history during beat drops)

---

### 3. Raymarched & Volumetric 3D FX

#### **SDF Crystal Displacive Shell (`crystal-lattice`)**
* **Concept:** Raymarches a 3D translucent crystal or polyhedron lattice whose surface textures and refraction maps are driven by the live video feed.
* **Technique:** 1-pass raymarching with Signed Distance Functions (SDF). Uses live frame UV sampling to map color, emission, and normal reflections onto ray intersections.
* **Parameters:**
  * `lattice_type` (Choice: Icosahedron, Cubical Grid, Menger Sponge)
  * `refraction` (Index of refraction warping)
  * `fresnel` (Edge glow intensity)
  * `dispersion` (Spectral chromatic splitting inside crystals)

#### **Volumetric Reaction Diffusion (`reaction-volumetrics`)**
* **Concept:** Generates organic, bioluminescent coral-like or zebra-stripe patterns that grow over and react to the live video input.
* **Technique:** **2-pass effect** leveraging history.
  * *Pass 0:* Gray-Scott reaction-diffusion Laplacian step using `custom_history_texture`.
  * *Pass 1:* Volumetric ray-march through the chemical concentration field with lighting and video overlay.
* **Parameters:**
  * `feed_rate` (Reaction parameter F)
  * `kill_rate` (Reaction parameter K)
  * `diffusion_ratio` (Pattern sharpness)
  * `video_blend` (Input source influence on reaction triggers)

---

### 4. Generative & Psychedelic Feedback

#### **Hyper-Dimensional Kaleidoscope (`hyper-kaleido`)**
* **Concept:** Higher-dimensional hyperbolic tiling and Mobius transformations that slice, mirror, and tessellate video into hypnotic mandalas.
* **Technique:** Complex plane conformal mapping ($z \to \frac{az+b}{cz+d}$) + polar space rotational foldings.
* **Parameters:**
  * `folds` (Number of symmetry axes)
  * `mobius_a`, `mobius_b` (Conformal transformation controls)
  * `spin` (Rotation speed)
  * `zoom` (Logarithmic spiral zoom)

#### **Spectral Thermal Vision (`spectral-thermal`)**
* **Concept:** Maps video luminance, edge detail, and history changes to custom false-color thermal palettes (e.g., FLIR, Rainbow, Neon Twilight, Cyberpunk).
* **Technique:** Multi-tap gradient analysis + palette ramp LUT interpolation + high-pass edge boost.
* **Parameters:**
  * `palette` (Choice: FLIR Ironbow, Inferno, Cyberpunk, Plasma, Monochromatic Edge)
  * `contrast_stretch` (Dynamic range mapping)
  * `contour_lines` (Isosurface topographic lines)
  * `heat_bleed` (Thermal diffusion bloom)

---

### Recommended Implementation Plan for the Effect Agent

For the implementation agent, creating an effect package consists of adding a subfolder in effects with:
1. effect.json specifying ID, label, parameters with controls (`slider`, `choice`, `toggle`), and pass entries (see format in docs/EFFECT_PACKAGES.md).
2. `<effect_name>.wgsl` implementing `vs_main` and the fragment entry points conforming to the `master-v1` uniform layout and group 0 bindings (see docs/EFFECT_PACKAGES.md).

Created 4 todos