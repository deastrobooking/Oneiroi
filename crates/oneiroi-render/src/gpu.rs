//! Device, queue, and swapchain surface.

use anyhow::{Context, Result};

/// Owns the GPU device and the window surface it presents to.
pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    operator: PresentSurface,
    adapter_info: wgpu::AdapterInfo,
}

/// One presentable window surface backed by the shared GPU device.
pub struct PresentSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceAcquireStatus {
    Healthy,
    Suboptimal,
    Outdated,
    Lost,
    Timeout,
    Occluded,
    Validation,
}

pub struct SurfaceAcquisition {
    pub frame: Option<wgpu::SurfaceTexture>,
    pub status: SurfaceAcquireStatus,
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
        let operator = PresentSurface::new(surface, &device, &caps, width, height);

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            operator,
            adapter_info,
        })
    }

    pub fn create_surface(
        &self,
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<PresentSurface> {
        let surface = self
            .instance
            .create_surface(target)
            .context("create additional window surface")?;
        let caps = surface.get_capabilities(&self.adapter);
        Ok(PresentSurface::new(
            surface,
            &self.device,
            &caps,
            width,
            height,
        ))
    }

    /// Gamma-space swapchain format. What the UI renders into.
    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.operator.surface_format()
    }

    /// sRGB-encoding view format. What clips and effects render into.
    pub fn content_format(&self) -> wgpu::TextureFormat {
        self.operator.content_format()
    }

    /// A view of `texture` that encodes linear shader output to sRGB.
    pub fn content_view(&self, texture: &wgpu::Texture) -> wgpu::TextureView {
        self.operator.content_view(texture)
    }

    /// A view of `texture` in the swapchain's own gamma-space format.
    pub fn surface_view(&self, texture: &wgpu::Texture) -> wgpu::TextureView {
        self.operator.surface_view(texture)
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
        self.operator.size()
    }

    pub fn aspect(&self) -> f32 {
        self.operator.aspect()
    }

    /// Reconfigure the swapchain. A zero dimension (minimised window) is
    /// ignored — configuring a zero-sized surface is a validation error.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.operator.resize(&self.device, width, height);
    }

    pub fn acquire(&mut self) -> Option<wgpu::SurfaceTexture> {
        self.operator.acquire(&self.device)
    }
}

impl PresentSurface {
    fn new(
        surface: wgpu::Surface<'static>,
        device: &wgpu::Device,
        caps: &wgpu::SurfaceCapabilities,
        width: u32,
        height: u32,
    ) -> Self {
        // Configure the swapchain in gamma space and expose its sRGB variant
        // as a view. Program rendering uses that view so blending remains in
        // linear light; egui uses the plain gamma-space surface view.
        let gamma_format = caps
            .formats
            .iter()
            .copied()
            .map(|format| format.remove_srgb_suffix())
            .find(|format| format.add_srgb_suffix() != *format && caps.formats.contains(format))
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
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: caps.alpha_modes[0],
            view_formats: if content_format == gamma_format {
                vec![]
            } else {
                vec![content_format]
            },
            desired_maximum_frame_latency: 2,
        };
        surface.configure(device, &config);
        Self { surface, config }
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn content_format(&self) -> wgpu::TextureFormat {
        self.config
            .view_formats
            .first()
            .copied()
            .unwrap_or(self.config.format)
    }

    pub fn content_view(&self, texture: &wgpu::Texture) -> wgpu::TextureView {
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("present-content-view"),
            format: Some(self.content_format()),
            ..Default::default()
        })
    }

    pub fn surface_view(&self, texture: &wgpu::Texture) -> wgpu::TextureView {
        texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("present-surface-view"),
            format: Some(self.surface_format()),
            ..Default::default()
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if (width, height) == (self.config.width, self.config.height) {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(device, &self.config);
    }

    /// Acquire the next swapchain texture, recovering from a stale surface.
    ///
    /// `Outdated`/`Lost` happen routinely when a window moves between displays
    /// — the exact thing that happens when the output window is dragged to the
    /// projector — so reconfigure and let the frame drop rather than panicking.
    pub fn acquire(&mut self, device: &wgpu::Device) -> Option<wgpu::SurfaceTexture> {
        self.acquire_with_status(device).frame
    }

    /// Acquire while retaining the exact swapchain health signal for
    /// operator diagnostics.
    pub fn acquire_with_status(&mut self, device: &wgpu::Device) -> SurfaceAcquisition {
        use wgpu::CurrentSurfaceTexture as Cst;

        match self.surface.get_current_texture() {
            Cst::Success(frame) => SurfaceAcquisition {
                frame: Some(frame),
                status: SurfaceAcquireStatus::Healthy,
            },
            Cst::Suboptimal(frame) => {
                // Usable this frame; reconfigure so the next one isn't.
                self.surface.configure(device, &self.config);
                SurfaceAcquisition {
                    frame: Some(frame),
                    status: SurfaceAcquireStatus::Suboptimal,
                }
            }
            Cst::Outdated => {
                self.surface.configure(device, &self.config);
                SurfaceAcquisition {
                    frame: None,
                    status: SurfaceAcquireStatus::Outdated,
                }
            }
            Cst::Lost => {
                self.surface.configure(device, &self.config);
                SurfaceAcquisition {
                    frame: None,
                    status: SurfaceAcquireStatus::Lost,
                }
            }
            Cst::Timeout => SurfaceAcquisition {
                frame: None,
                status: SurfaceAcquireStatus::Timeout,
            },
            Cst::Occluded => SurfaceAcquisition {
                frame: None,
                status: SurfaceAcquireStatus::Occluded,
            },
            Cst::Validation => {
                log::error!("surface acquire hit a validation error");
                SurfaceAcquisition {
                    frame: None,
                    status: SurfaceAcquireStatus::Validation,
                }
            }
        }
    }
}
