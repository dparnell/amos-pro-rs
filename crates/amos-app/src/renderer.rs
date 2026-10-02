//! wgpu renderer: composites the AMOS screens of a [`Frame`] onto a virtual
//! Amiga display texture, then scales that onto the window.

use std::collections::HashMap;
use std::sync::Arc;

use amos_core::display::{DISPLAY_HEIGHT, DISPLAY_WIDTH, Frame, Layer, LayerFormat};
use bytemuck::{Pod, Zeroable};
use winit::window::Window;

const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const PALETTE_WIDTH: u32 = 256;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LayerUniforms {
    band: [f32; 4],
    window: [f32; 4],
    src: [f32; 4],
    info: [i32; 4],
    display: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlitUniforms {
    rect: [f32; 4],
    sizes: [f32; 4],
}

/// GPU resources kept for one AMOS screen between frames.
struct LayerResources {
    width: u32,
    height: u32,
    format: LayerFormat,
    palette_rows: u32,
    pixels_version: u64,
    pixels: wgpu::Texture,
    palette: wgpu::Texture,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    used: bool,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    layer_pipeline: wgpu::RenderPipeline,
    layer_bind_layout: wgpu::BindGroupLayout,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group: wgpu::BindGroup,
    blit_uniforms: wgpu::Buffer,
    display_view: wgpu::TextureView,
    dummy_indexed: wgpu::TextureView,
    dummy_rgba: wgpu::TextureView,
    layers: HashMap<u32, LayerResources>,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window).map_err(|e| e.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                force_fallback_adapter: false,
                compatible_surface: Some(&surface),
                apply_limit_buckets: false,
            })
            .await
            .map_err(|e| e.to_string())?;
        log::info!("Using graphics adapter {:?}", adapter.get_info());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("amos device"),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;

        // AMOS colours are raw 12 bit RGB values, so present them without any
        // sRGB conversion when the surface allows it.
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::include_wgsl!("shaders.wgsl"));

        let layer_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("layer bind layout"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, wgpu::TextureSampleType::Uint),
                texture_entry(2, wgpu::TextureSampleType::Float { filterable: false }),
                texture_entry(3, wgpu::TextureSampleType::Float { filterable: false }),
            ],
        });
        let layer_pipeline = create_pipeline(
            &device,
            &shader,
            &layer_bind_layout,
            "layer_vs",
            "layer_fs",
            DISPLAY_FORMAT,
        );

        let blit_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit bind layout"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, wgpu::TextureSampleType::Float { filterable: true }),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_pipeline = create_pipeline(&device, &shader, &blit_bind_layout, "blit_vs", "blit_fs", format);

        let display = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("amiga display"),
            size: wgpu::Extent3d { width: DISPLAY_WIDTH, height: DISPLAY_HEIGHT, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DISPLAY_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let display_view = display.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("display sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blit_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("blit uniforms"),
            size: size_of::<BlitUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit bind group"),
            layout: &blit_bind_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: blit_uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&display_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let dummy_indexed = create_texture(&device, 1, 1, wgpu::TextureFormat::R8Uint, "dummy indexed")
            .create_view(&Default::default());
        let dummy_rgba = create_texture(&device, 1, 1, wgpu::TextureFormat::Rgba8Unorm, "dummy rgba")
            .create_view(&Default::default());

        Ok(Self {
            surface,
            device,
            queue,
            config,
            layer_pipeline,
            layer_bind_layout,
            blit_pipeline,
            blit_bind_group,
            blit_uniforms,
            display_view,
            dummy_indexed,
            dummy_rgba,
            layers: HashMap::new(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Rectangle (x, y, w, h) in window pixels where the display is shown.
    pub fn display_rect(&self) -> [f32; 4] {
        let (sw, sh) = (self.config.width as f32, self.config.height as f32);
        let (dw, dh) = (DISPLAY_WIDTH as f32, DISPLAY_HEIGHT as f32);
        let mut scale = (sw / dw).min(sh / dh);
        if scale >= 1.0 {
            // Prefer integer scales when they fill most of the window.
            let int_scale = scale.floor();
            if int_scale / scale > 0.9 {
                scale = int_scale;
            }
        }
        let (w, h) = ((dw * scale).round(), (dh * scale).round());
        [((sw - w) / 2.0).floor(), ((sh - h) / 2.0).floor(), w, h]
    }

    pub fn render(&mut self, frame: &Frame) {
        for res in self.layers.values_mut() {
            res.used = false;
        }
        let mut order = Vec::with_capacity(frame.layers.len());
        for layer in &frame.layers {
            self.upload_layer(layer);
            order.push(layer.id);
        }
        self.layers.retain(|_, res| res.used);

        let surface_texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            other => {
                log::debug!("Skipping frame: {:?}", std::mem::discriminant(&other));
                return;
            }
        };
        let surface_view = surface_texture.texture.create_view(&Default::default());

        let rect = self.display_rect();
        self.queue.write_buffer(
            &self.blit_uniforms,
            0,
            bytemuck::bytes_of(&BlitUniforms {
                rect,
                sizes: [
                    DISPLAY_WIDTH as f32,
                    DISPLAY_HEIGHT as f32,
                    self.config.width as f32,
                    self.config.height as f32,
                ],
            }),
        );

        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let border = frame.border.map(|c| c as f64 / 255.0);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite screens"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.display_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: border[0], g: border[1], b: border[2], a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.layer_pipeline);
            for id in order {
                pass.set_bind_group(0, &self.layers[&id].bind_group, &[]);
                pass.draw(0..4, 0..1);
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scale display"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &self.blit_bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(surface_texture);
    }

    fn upload_layer(&mut self, layer: &Layer) {
        let palette_rows = layer.palette_rows.max(1);
        let stale = match self.layers.get(&layer.id) {
            Some(res) => {
                res.width != layer.width
                    || res.height != layer.height
                    || res.format != layer.format
                    || res.palette_rows != palette_rows
            }
            None => true,
        };
        if stale {
            let res = self.create_layer_resources(layer, palette_rows);
            self.layers.insert(layer.id, res);
        }
        let res = self.layers.get_mut(&layer.id).unwrap();
        res.used = true;

        if stale || res.pixels_version != layer.pixels_version {
            let bpp = match layer.format {
                LayerFormat::Indexed => 1,
                LayerFormat::Rgba => 4,
            };
            write_texture(&self.queue, &res.pixels, layer.pixels, layer.width, layer.height, bpp);
            res.pixels_version = layer.pixels_version;
        }

        // Palettes are small: always upload them so fades, colour cycling and
        // rainbows take effect immediately.
        let mut palette = vec![[0u8; 4]; (PALETTE_WIDTH * palette_rows) as usize];
        for row in 0..palette_rows as usize {
            let src = &layer.palette[(row * PALETTE_WIDTH as usize).min(layer.palette.len())..];
            let n = src.len().min(PALETTE_WIDTH as usize);
            palette[row * PALETTE_WIDTH as usize..][..n].copy_from_slice(&src[..n]);
        }
        write_texture(&self.queue, &res.palette, bytemuck::cast_slice(&palette), PALETTE_WIDTH, palette_rows, 4);

        let rect = |r: amos_core::display::Rect| [r.x as f32, r.y as f32, r.w as f32, r.h as f32];
        let uniforms = LayerUniforms {
            band: rect(layer.band),
            window: rect(layer.window),
            src: [layer.src_x as f32, layer.src_y as f32, layer.scale_x.max(1) as f32, layer.scale_y.max(1) as f32],
            info: [
                layer.width as i32,
                layer.height as i32,
                palette_rows as i32,
                layer.transparent.map_or(-1, i32::from),
            ],
            display: [
                DISPLAY_WIDTH as f32,
                DISPLAY_HEIGHT as f32,
                if layer.format == LayerFormat::Rgba { 1.0 } else { 0.0 },
                0.0,
            ],
        };
        self.queue.write_buffer(&res.uniforms, 0, bytemuck::bytes_of(&uniforms));
    }

    fn create_layer_resources(&self, layer: &Layer, palette_rows: u32) -> LayerResources {
        let (w, h) = (layer.width.max(1), layer.height.max(1));
        let texture_format = match layer.format {
            LayerFormat::Indexed => wgpu::TextureFormat::R8Uint,
            LayerFormat::Rgba => wgpu::TextureFormat::Rgba8Unorm,
        };
        let pixels = create_texture(&self.device, w, h, texture_format, "screen pixels");
        let palette = create_texture(&self.device, PALETTE_WIDTH, palette_rows, wgpu::TextureFormat::Rgba8Unorm, "palette");
        let uniforms = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer uniforms"),
            size: size_of::<LayerUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pixels_view = pixels.create_view(&Default::default());
        let palette_view = palette.create_view(&Default::default());
        let (indexed_view, rgba_view) = match layer.format {
            LayerFormat::Indexed => (&pixels_view, &self.dummy_rgba),
            LayerFormat::Rgba => (&self.dummy_indexed, &pixels_view),
        };
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("layer bind group"),
            layout: &self.layer_bind_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(indexed_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(rgba_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&palette_view) },
            ],
        });
        LayerResources {
            width: layer.width,
            height: layer.height,
            format: layer.format,
            palette_rows,
            pixels_version: 0,
            pixels,
            palette,
            uniforms,
            bind_group,
            used: true,
        }
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
    vs: &str,
    fs: &str,
    format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(vs),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn create_texture(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write_texture(queue: &wgpu::Queue, texture: &wgpu::Texture, data: &[u8], w: u32, h: u32, bpp: u32) {
    let needed = (w * h * bpp) as usize;
    if data.len() < needed || w == 0 || h == 0 {
        return;
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data[..needed],
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * bpp), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}
