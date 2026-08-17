use shaderc::{CompileOptions, Compiler, ShaderKind};

pub struct PipelineManager {
    compiler: Compiler,
}

impl PipelineManager {
    pub fn new() -> Self {
        Self {
            compiler: Compiler::new().expect("Failed to initialize shaderc compiler"),
        }
    }

    /// Compiles GLSL fragment shader source into SPIR-V and creates a `wgpu::ShaderModule`.
    /// If compilation fails, prints error trace to stderr and returns `Err`.
    pub fn compile_fragment_shader(
        &mut self,
        device: &wgpu::Device,
        glsl_source: &str,
    ) -> Result<wgpu::ShaderModule, String> {
        let mut options = CompileOptions::new().ok_or("Failed to create compile options")?;
        options.set_target_env(
            shaderc::TargetEnv::Vulkan,
            shaderc::EnvVersion::Vulkan1_0 as u32,
        );

        let compilation_artifact = self.compiler.compile_into_spirv(
            glsl_source,
            ShaderKind::Fragment,
            "shader.frag",
            "main",
            Some(&options),
        );

        match compilation_artifact {
            Ok(artifact) => {
                let spirv_binary = artifact.as_binary_u8();
                let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Hot-Reloaded Fragment Shader"),
                    source: wgpu::util::make_spirv(spirv_binary),
                });
                Ok(shader_module)
            }
            Err(err) => {
                eprintln!("\n========== [GLSL COMPILATION ERROR] ==========\n{}\n================================================\n", err);
                Err(err.to_string())
            }
        }
    }

    /// Creates or updates a `wgpu::RenderPipeline` using the provided GLSL fragment shader source.
    /// Retains `previous_pipeline` if compilation of `glsl_source` fails.
    pub fn create_or_update_pipeline(
        &mut self,
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        format: wgpu::TextureFormat,
        glsl_source: &str,
        vertex_module: &wgpu::ShaderModule,
        previous_pipeline: Option<std::sync::Arc<wgpu::RenderPipeline>>,
    ) -> std::sync::Arc<wgpu::RenderPipeline> {
        match self.compile_fragment_shader(device, glsl_source) {
            Ok(frag_module) => {
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Quasar Render Pipeline"),
                    layout: Some(pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: vertex_module,
                        entry_point: "main",
                        buffers: &[Vertex::layout()],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &frag_module,
                        entry_point: "main",
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: Some(wgpu::BlendState::REPLACE),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        unclipped_depth: false,
                        polygon_mode: wgpu::PolygonMode::Fill,
                        conservative: false,
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                });
                log::info!("Successfully compiled and created new RenderPipeline.");
                std::sync::Arc::new(pipeline)
            }
            Err(_) => {
                log::warn!("Retaining previous valid RenderPipeline due to GLSL compilation failure.");
                previous_pipeline.expect("No valid initial RenderPipeline available to retain!")
            }
        }
    }
}

/// Fullscreen quad vertex layout
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
}

impl Vertex {
    pub fn layout<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            }],
        }
    }
}

pub const QUAD_VERTICES: &[Vertex] = &[
    Vertex { position: [-1.0, -1.0] },
    Vertex { position: [1.0, -1.0] },
    Vertex { position: [-1.0, 1.0] },
    Vertex { position: [-1.0, 1.0] },
    Vertex { position: [1.0, -1.0] },
    Vertex { position: [1.0, 1.0] },
];
