use notify::{EventKind, RecursiveMode, Watcher};
use std::sync::Arc;
use std::time::Instant;
use wgpu::util::DeviceExt;
use winit::{
    event::*,
    event_loop::EventLoop,
    window::WindowBuilder,
};

mod audio;


const VERTEX_SHADER_GLSL: &str = r#"
#version 450

layout(location = 0) out vec2 v_uv;

void main() {
    // Quad formed by two triangles (6 vertices)
    vec2 positions[6] = vec2[](
        vec2(-1.0, -1.0),
        vec2( 1.0, -1.0),
        vec2(-1.0,  1.0),
        vec2(-1.0,  1.0),
        vec2( 1.0, -1.0),
        vec2( 1.0,  1.0)
    );

    vec2 uvs[6] = vec2[](
        vec2(0.0, 1.0),
        vec2(1.0, 1.0),
        vec2(0.0, 0.0),
        vec2(0.0, 0.0),
        vec2(1.0, 1.0),
        vec2(1.0, 0.0)
    );

    gl_Position = vec4(positions[gl_VertexIndex], 0.0, 1.0);
    v_uv = uvs[gl_VertexIndex];
}
"#;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    time: f32,
    _padding: f32,
    resolution: [f32; 2],
}

fn compile_glsl_to_wgsl(source: &str, stage: naga::ShaderStage) -> Result<String, Box<dyn std::error::Error>> {
    let mut frontend = naga::front::glsl::Frontend::default();
    let options = naga::front::glsl::Options {
        stage,
        defines: Default::default(),
    };
    let module = frontend
        .parse(&options, source)
        .map_err(|errs| format!("GLSL parse error: {:?}", errs))?;

    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    let info = validator.validate(&module)?;

    let wgsl = naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty())?;
    Ok(wgsl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_naga_glsl_sampler() {
        let glsl = r#"#version 450
layout(location = 0) out vec4 fragColor;
layout(set = 0, binding = 1) uniform texture2D u_audio;
layout(set = 0, binding = 2) uniform sampler u_audio_sampler;
void main() {
    fragColor = texture(sampler2D(u_audio, u_audio_sampler), vec2(0.5, 0.5));
}
"#;
        let res = compile_glsl_to_wgsl(glsl, naga::ShaderStage::Fragment);
        println!("Result: {:?}", res);
        assert!(res.is_ok());
    }


}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let audio_system = match audio::init_audio() {
        Ok(sys) => {
            println!("[Audio] Successfully initialized audio input and FFT processing.");
            Some(sys)
        }
        Err(err) => {
            eprintln!("[Audio Warning] Could not start audio capture: {}", err);
            None
        }
    };

    let event_loop = EventLoop::new()?;

    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Quasar - Naga GLSL Hot-Reloading Shader")
            .with_inner_size(winit::dpi::LogicalSize::new(800, 600))
            .build(&event_loop)?,
    );

    let instance = wgpu::Instance::default();
    let surface = instance.create_surface(Arc::clone(&window))?;

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))
    .ok_or("Failed to find an appropriate graphics adapter")?;

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("Quasar Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))?;

    let size = window.inner_size();
    let surface_caps = surface.get_capabilities(&adapter);
    let surface_format = surface_caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(surface_caps.formats[0]);

    let present_mode = surface_caps
        .present_modes
        .iter()
        .copied()
        .find(|&mode| mode == wgpu::PresentMode::Fifo)
        .unwrap_or(surface_caps.present_modes[0]);

    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode,
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    // Uniform buffer for animated shader parameter
    let initial_globals = Globals {
        time: 0.0,
        _padding: 0.0,
        resolution: [size.width as f32, size.height as f32],
    };

    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Globals Uniform Buffer"),
        contents: bytemuck::bytes_of(&initial_globals),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    // 2D texture (512x1) for 512 FFT frequency floats
    let audio_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Audio FFT Texture"),
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

    let audio_texture_view = audio_texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2),
        ..Default::default()
    });

    let audio_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Audio FFT Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Globals and Audio Bind Group Layout"),
        entries: &[
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
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },

            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                count: None,
            },
        ],
    });


    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Globals and Audio Bind Group"),
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&audio_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&audio_sampler),
            },
        ],
    });

    // Load initial GLSL fragment shader from shaders/test.glsl
    let shader_path = "shaders/test.glsl";
    println!("Loading fragment shader from {}", shader_path);
    let fs_glsl = std::fs::read_to_string(shader_path)?;

    // Compile GLSL to WGSL using Naga
    let vs_wgsl = compile_glsl_to_wgsl(VERTEX_SHADER_GLSL, naga::ShaderStage::Vertex)?;
    let fs_wgsl = compile_glsl_to_wgsl(&fs_glsl, naga::ShaderStage::Fragment)?;

    let vs_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Vertex Shader Module"),
        source: wgpu::ShaderSource::Wgsl(vs_wgsl.into()),
    });

    let fs_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Fragment Shader Module"),
        source: wgpu::ShaderSource::Wgsl(fs_wgsl.into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Pipeline Layout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let mut render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Render Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &vs_module,
            entry_point: "main",
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &fs_module,
            entry_point: "main",
            targets: &[Some(wgpu::ColorTargetState {
                format: config.format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    // Setup notify file watcher to watch the shaders directory
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
        if let Ok(event) = res {
            if matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                let _ = tx.send(());
            }
        }
    })?;
    watcher.watch(std::path::Path::new("shaders"), RecursiveMode::Recursive)?;

    let start_time = Instant::now();

    event_loop.run(move |event, elwt| {
        let _watcher_ref = &watcher;
        match event {
            Event::WindowEvent { event, window_id } if window_id == window.id() => match event {
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::Resized(new_size) => {
                    if new_size.width > 0 && new_size.height > 0 {
                        config.width = new_size.width;
                        config.height = new_size.height;
                        surface.configure(&device, &config);
                        window.request_redraw();
                    }
                }
                WindowEvent::RedrawRequested => {
                    let elapsed = start_time.elapsed().as_secs_f32();
                    let globals = Globals {
                        time: elapsed,
                        _padding: 0.0,
                        resolution: [config.width as f32, config.height as f32],
                    };
                    queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&globals));

                    // Lock and read 512 frequency floats from AudioSystem
                    let audio_data = if let Some(ref sys) = audio_system {
                        if let Ok(lock) = sys.fft_spectrum.lock() {
                            *lock
                        } else {
                            [0.0f32; 512]
                        }
                    } else {
                        [0.0f32; 512]
                    };

                    // Upload FFT spectrum to 1D texture
                    queue.write_texture(
                        wgpu::ImageCopyTexture {
                            texture: &audio_texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        bytemuck::cast_slice(&audio_data),
                        wgpu::ImageDataLayout {
                            offset: 0,
                            bytes_per_row: Some(512 * 4),
                            rows_per_image: Some(1),
                        },
                        wgpu::Extent3d {
                            width: 512,
                            height: 1,
                            depth_or_array_layers: 1,
                        },
                    );


                    match surface.get_current_texture() {
                        Ok(output) => {
                            let view = output
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());
                            let mut encoder =
                                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("Render Encoder"),
                                });

                            {
                                let mut render_pass =
                                    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                        label: Some("Render Pass"),
                                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                            view: &view,
                                            resolve_target: None,
                                            ops: wgpu::Operations {
                                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                                store: wgpu::StoreOp::Store,
                                            },
                                        })],
                                        depth_stencil_attachment: None,
                                        timestamp_writes: None,
                                        occlusion_query_set: None,
                                    });

                                render_pass.set_pipeline(&render_pipeline);
                                render_pass.set_bind_group(0, &bind_group, &[]);
                                render_pass.draw(0..6, 0..1);
                            }

                            queue.submit(std::iter::once(encoder.finish()));
                            output.present();
                        }
                        Err(wgpu::SurfaceError::Outdated) => {
                            surface.configure(&device, &config);
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => elwt.exit(),
                        Err(e) => eprintln!("Surface error: {:?}", e),
                    }
                }
                _ => {}
            },
            Event::AboutToWait => {
                // Check if any shader modification events were triggered
                if rx.try_recv().is_ok() {
                    // Drain remaining events in channel (debounce)
                    while rx.try_recv().is_ok() {}

                    println!("[Hot-Reload] Change detected in shaders/, recompiling test.glsl...");
                    match std::fs::read_to_string("shaders/test.glsl") {
                        Ok(new_fs_glsl) => {
                            match compile_glsl_to_wgsl(&new_fs_glsl, naga::ShaderStage::Fragment) {
                                Ok(new_fs_wgsl) => {
                                    let new_fs_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                                        label: Some("Fragment Shader Module (Hot-reloaded)"),
                                        source: wgpu::ShaderSource::Wgsl(new_fs_wgsl.into()),
                                    });

                                    let new_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                                        label: Some("Render Pipeline (Hot-reloaded)"),
                                        layout: Some(&pipeline_layout),
                                        vertex: wgpu::VertexState {
                                            module: &vs_module,
                                            entry_point: "main",
                                            buffers: &[],
                                        },
                                        fragment: Some(wgpu::FragmentState {
                                            module: &new_fs_module,
                                            entry_point: "main",
                                            targets: &[Some(wgpu::ColorTargetState {
                                                format: config.format,
                                                blend: Some(wgpu::BlendState::REPLACE),
                                                write_mask: wgpu::ColorWrites::ALL,
                                            })],
                                        }),
                                        primitive: wgpu::PrimitiveState {
                                            topology: wgpu::PrimitiveTopology::TriangleList,
                                            strip_index_format: None,
                                            front_face: wgpu::FrontFace::Ccw,
                                            cull_mode: None,
                                            polygon_mode: wgpu::PolygonMode::Fill,
                                            unclipped_depth: false,
                                            conservative: false,
                                        },
                                        depth_stencil: None,
                                        multisample: wgpu::MultisampleState::default(),
                                        multiview: None,
                                    });

                                    render_pipeline = new_pipeline;
                                    println!("[Hot-Reload] Successfully recompiled and reloaded shaders/test.glsl!");
                                }
                                Err(err) => {
                                    eprintln!("[Hot-Reload Error] Failed to compile shaders/test.glsl:\n{}", err);
                                }
                            }
                        }
                        Err(err) => {
                            eprintln!("[Hot-Reload Error] Failed to read shaders/test.glsl: {}", err);
                        }
                    }
                }

                window.request_redraw();
            }
            _ => {}
        }
    })?;

    Ok(())
}

