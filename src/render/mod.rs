pub mod pipeline;

use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::audio::AudioHandle;
use crate::render::pipeline::{PipelineManager, QUAD_VERTICES};

pub const DEFAULT_FRAGMENT_SHADER: &str = r#"#version 450
layout(location = 0) out vec4 fragColor;

layout(set = 0, binding = 0) uniform Uniforms {
    vec2 u_resolution;
    float u_time;
    float u_volume;
    vec4 u_palette[4];
};

layout(set = 0, binding = 1) uniform texture2D u_spectrum;
layout(set = 0, binding = 2) uniform sampler u_sampler;

void main() {
    vec2 uv = gl_FragCoord.xy / u_resolution;
    float spec = texture(sampler2D(u_spectrum, u_sampler), vec2(uv.x, 0.5)).r;
    
    vec3 base_col = mix(u_palette[0].rgb, u_palette[1].rgb, uv.x);
    vec3 wave_col = mix(u_palette[2].rgb, u_palette[3].rgb, spec);
    
    float bar = step(uv.y, spec * 0.8 * (0.2 + u_volume));
    vec3 final_color = mix(base_col, wave_col, bar);
    
    fragColor = vec4(final_color, 1.0);
}
"#;

pub const DEFAULT_VERTEX_SHADER: &str = r#"#version 450
layout(location = 0) in vec2 a_position;

void main() {
    gl_Position = vec4(a_position, 0.0, 1.0);
}
"#;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    pub u_resolution: [f32; 2],
    pub u_time: f32,
    pub u_volume: f32,
    pub u_palette: [[f32; 4]; 4],
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
            u_resolution: [800.0, 600.0],
            u_time: 0.0,
            u_volume: 0.0,
            u_palette: [
                [0.11, 0.53, 0.89, 1.0],   // Primary
                [0.61, 0.35, 0.71, 1.0],   // Secondary
                [0.07, 0.07, 0.09, 1.0],   // Background
                [0.95, 0.76, 0.20, 1.0],   // Accent
            ],
        }
    }
}

pub struct Renderer {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
    pub vertex_buffer: wgpu::Buffer,
    pub uniform_buffer: wgpu::Buffer,
    pub uniforms: Uniforms,
    pub spectrum_texture: wgpu::Texture,
    pub spectrum_texture_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    pub pipeline_layout: wgpu::PipelineLayout,
    pub render_pipeline: std::sync::Arc<wgpu::RenderPipeline>,
    pub vertex_module: wgpu::ShaderModule,
    pub pipeline_manager: PipelineManager,
    pub audio_handle: AudioHandle,
    pub start_time: std::time::Instant,
}

impl Renderer {
    pub async fn new(
        window: Arc<Window>,
        audio_handle: AudioHandle,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone())?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or("Failed to find suitable GPU adapter")?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Quasar GPU Device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                },
                None,
            )
            .await?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo, // Cap at 60 FPS / VSync
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Vertex Buffer (fullscreen quad)
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Fullscreen Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Uniform Buffer (set = 0, binding = 0)
        let mut uniforms = Uniforms::default();
        uniforms.u_resolution = [size.width as f32, size.height as f32];

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Audio Spectrum Texture (set = 0, binding = 1) -> 512x1 R32Float texture
        let spectrum_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Audio Spectrum 1D Texture"),
            size: wgpu::Extent3d {
                width: 512,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let spectrum_texture_view =
            spectrum_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Sampler (set = 0, binding = 2)
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Spectrum Texture Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Bind Group Layout
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Quasar Bind Group Layout"),
                entries: &[
                    // binding = 0: Uniforms
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // binding = 1: Audio Spectrum Texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // binding = 2: Sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Quasar Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&spectrum_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let mut pipeline_manager = PipelineManager::new();

        // Compile vertex shader
        let vertex_module = pipeline_manager
            .compile_fragment_shader(&device, DEFAULT_VERTEX_SHADER)
            .or_else(|_| {
                // Fallback vertex module using WGSL if GLSL vertex fails
                Ok::<_, String>(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Fallback Vertex Shader WGSL"),
                    source: wgpu::ShaderSource::Wgsl(
                        r#"
                        struct VertexInput {
                            @location(0) position: vec2<f32>,
                        };
                        @vertex
                        fn main(input: VertexInput) -> @builtin(position) vec4<f32> {
                            return vec4<f32>(input.position, 0.0, 1.0);
                        }
                    "#
                        .into(),
                    ),
                }))
            })
            .unwrap();

        let render_pipeline = pipeline_manager.create_or_update_pipeline(
            &device,
            &pipeline_layout,
            config.format,
            DEFAULT_FRAGMENT_SHADER,
            &vertex_module,
            None,
        );

        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            vertex_buffer,
            uniform_buffer,
            uniforms,
            spectrum_texture,
            spectrum_texture_view,
            sampler,
            bind_group_layout,
            bind_group,
            pipeline_layout,
            render_pipeline,
            vertex_module,
            pipeline_manager,
            audio_handle,
            start_time: std::time::Instant::now(),
        })
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
            self.uniforms.u_resolution = [new_size.width as f32, new_size.height as f32];
        }
    }

    /// Hot-reload the fragment shader from GLSL string source code.
    pub fn reload_fragment_shader(&mut self, glsl_source: &str) {
        let old_pipeline = Some(self.render_pipeline.clone());
        self.render_pipeline = self.pipeline_manager.create_or_update_pipeline(
            &self.device,
            &self.pipeline_layout,
            self.config.format,
            glsl_source,
            &self.vertex_module,
            old_pipeline,
        );
    }

    /// Updates frame uniforms & spectrum texture, then draws the frame.
    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Update uniforms: time & volume
        self.uniforms.u_time = self.start_time.elapsed().as_secs_f32();
        self.uniforms.u_volume = self.audio_handle.get_volume();

        self.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[self.uniforms]),
        );

        // Update 1D Spectrum Texture with latest 512 spectrum bins
        if let Ok(spec_guard) = self.audio_handle.spectrum.lock() {
            self.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &self.spectrum_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&*spec_guard),
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(512 * std::mem::size_of::<f32>() as u32),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 512,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Quasar Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Quasar Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.05,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.draw(0..6, 0..1);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
