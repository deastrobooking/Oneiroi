//! Device, queue, and swapchain surface.

use anyhow::{Context, Result};

/// Owns the GPU device and the window surface it presents to.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    adapter_info: wgpu::AdapterInfo,
}

impl Gpu {
    /// Create a device and configure a surface for `target` at `width`x`height`
    /// physical pixels.
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        // No display handle: it is only consulted by the GL backend, and a VJ
        // app has no business falling back to GL. Reading the env means
        // `WGPU_BACKEND=vulkan` etc. work for debugging.
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

        let surface = instance
            .create_surface(target)
            .context("create surface for window")?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .context("no GPU adapter supports this surface")?;

        let adapter_info = adapter.get_info();
        let adapter_features = adapter.features();
        let requested_features = adapter_features & wgpu::Features::TEXTURE_COMPRESSION_BC;
        log::info!(
            "adapter: {} ({:?}, {:?}); BC textures: {}",
            adapter_info.name,
            adapter_info.device_type,
            adapter_info.backend,
            requested_features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
        );

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("oneiroi-device"),
                required_features: requested_features,
                ..Default::default()
            })
            .await
            .context("request device")?;

        let caps = surface.get_capabilities(&adapter);

        // Colour space, decided once, here.
        //
        // Composited content is shaded in linear space and encoded to sRGB on
        // write — the only way blend modes and stacked effects stay correct.
        // egui, though, wants a gamma-space target and warns loudly otherwise.
        //
        // Both get what they want by configuring the swapchain in gamma space
        // and listing the sRGB variant as a view format: content renders
        // through an `*Srgb` view of the same texture, the UI through the plain
        // one. No manual pow(2.2) anywhere in this codebase.
        let gamma_format = caps
            .formats
            .iter()
            .copied()
            .map(|f| f.remove_srgb_suffix())
            .find(|f| f.add_srgb_suffix() != *f && caps.formats.contains(f))
            .unwrap_or(caps.formats[0]);
        let content_format = gamma_format.add_srgb_suffix();
        if content_format == gamma_format {
            log::warn!("no sRGB view format for {gamma_format:?}; content will render unencoded");
        }

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: gamma_format,
            width: width.max(1),
            height: height.max(1),
            // Vsync. Frame pacing gets its own clip clock later; presentation
            // stays locked to the display.
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: if content_format == gamma_format {
                vec![]
            } else {
                vec![content_format]
            },
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Ok(Self {
            device,
            queue,
            surface,
            config,
            adapter_info,
        })
    }

    /// Gamma-space swapchain format. What the UI renders into.
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    /// sRGB-encoding view format. What clips and effects render into.
    pub fn content_format(&self) -> wgpu::TextureFormat {
        self.config
            .view_formats
            .first()
            .copied()
            .unwrap_or(self.config.format)
    }

    /// A view of `texture` that encodes linear shader output to sRGB.
    pub fn content_view(&self, texture: &wgpu::Texture) -> wgpu::TextureView {
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("content-view"),
            format: Some(self.content_format()),
            ..Default::default()
        })
    }

    /// A view of `texture` in the swapchain's own gamma-space format.
    pub fn surface_view(&self, texture: &wgpu::Texture) -> wgpu::TextureView {
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("surface-view"),
            format: Some(self.surface_format()),
            ..Default::default()
        })
    }

    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.adapter_info
    }

    pub fn supports_bc_textures(&self) -> bool {
        self.device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    /// Reconfigure the swapchain. A zero dimension (minimised window) is
    /// ignored — configuring a zero-sized surface is a validation error.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if (width, height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Acquire the next swapchain texture, recovering from a stale surface.
    ///
    /// `Outdated`/`Lost` happen routinely when a window moves between displays
    /// — the exact thing that happens when the output window is dragged to the
    /// projector — so reconfigure and let the frame drop rather than panicking.
    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        use wgpu::CurrentSurfaceTexture as Cst;

        match self.surface.get_current_texture() {
            Cst::Success(frame) => Some(frame),
            Cst::Suboptimal(frame) => {
                // Usable this frame; reconfigure so the next one isn't.
                self.surface.configure(&self.device, &self.config);
                Some(frame)
            }
            Cst::Outdated | Cst::Lost => {
                self.surface.configure(&self.device, &self.config);
                None
            }
            Cst::Timeout | Cst::Occluded => None,
            Cst::Validation => {
                log::error!("surface acquire hit a validation error");
                None
            }
        }
    }
}
