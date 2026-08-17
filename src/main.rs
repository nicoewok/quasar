use std::fs;
use std::sync::Arc;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;

use quasar::audio::AudioSystem;
use quasar::config::{self, Theme};
use quasar::render::{Renderer, DEFAULT_FRAGMENT_SHADER};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    log::info!("Starting Quasar Visualizer...");

    // 1. Resolve and create configuration directories (~/.config/quasar or %APPDATA%/quasar)
    let config_paths = config::init_config_dirs()?;
    log::info!("Config directory: {:?}", config_paths.base_dir);

    // Copy default assets into user config directories if absent
    let default_shader_path = config_paths.shaders_dir.join("accretion.glsl");
    if !default_shader_path.exists() {
        if let Ok(asset_shader) = fs::read_to_string("assets/shaders/accretion.glsl") {
            let _ = fs::write(&default_shader_path, asset_shader);
        } else {
            let _ = fs::write(&default_shader_path, DEFAULT_FRAGMENT_SHADER);
        }
    }

    let default_theme_path = config_paths.themes_dir.join("monochrome_void.json");
    if !default_theme_path.exists() {
        if let Ok(asset_theme) = fs::read_to_string("assets/themes/monochrome_void.json") {
            let _ = fs::write(&default_theme_path, asset_theme);
        }
    }

    // 2. Setup directory watcher for hot-reloading shaders and themes
    let (_watcher, watcher_rx) = match config::watch_config_dirs(&config_paths) {
        Ok((w, rx)) => (Some(w), Some(rx)),
        Err(e) => {
            log::warn!("Failed to setup file watcher: {}", e);
            (None, None)
        }
    };

    // 3. Initialize real-time audio capture and DSP pipeline
    let audio_system = AudioSystem::new()?;
    log::info!("Real-time audio pipeline initialized.");

    // 4. Create Winit window and event loop
    let event_loop = EventLoop::new()?;
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Quasar - Real-Time Audio Visualizer")
            .with_inner_size(winit::dpi::PhysicalSize::new(1280, 720))
            .build(&event_loop)?,
    );

    // 5. Initialize GPU renderer with WGPU
    let mut renderer = pollster::block_on(Renderer::new(window.clone(), audio_system.handle))?;

    // Load initial shader and theme if present
    if default_shader_path.exists() {
        if let Ok(shader_src) = fs::read_to_string(&default_shader_path) {
            renderer.reload_fragment_shader(&shader_src);
        }
    }

    if default_theme_path.exists() {
        if let Ok(theme_src) = fs::read_to_string(&default_theme_path) {
            if let Ok(theme) = Theme::from_json(&theme_src) {
                renderer.uniforms.u_palette = [
                    theme.primary,
                    theme.secondary,
                    theme.background,
                    theme.accent,
                ];
            }
        }
    }

    // 6. Run Winit Event Loop
    event_loop.run(move |event, target| {
        target.set_control_flow(ControlFlow::Poll);

        // Check hot-reload file watcher receiver on every frame
        if let Some(ref rx) = watcher_rx {
            while let Ok(Ok(watch_event)) = rx.try_recv() {
                for path in watch_event.paths {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    match ext {
                        "glsl" | "frag" => {
                            log::info!("Hot-reloading shader: {:?}", path);
                            if let Ok(shader_src) = fs::read_to_string(&path) {
                                renderer.reload_fragment_shader(&shader_src);
                            }
                        }
                        "json" => {
                            log::info!("Hot-reloading theme: {:?}", path);
                            if let Ok(theme_src) = fs::read_to_string(&path) {
                                if let Ok(theme) = Theme::from_json(&theme_src) {
                                    renderer.uniforms.u_palette = [
                                        theme.primary,
                                        theme.secondary,
                                        theme.background,
                                        theme.accent,
                                    ];
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        match event {
            Event::WindowEvent { event, window_id } if window_id == window.id() => match event {
                WindowEvent::CloseRequested => {
                    target.exit();
                }
                WindowEvent::Resized(physical_size) => {
                    renderer.resize(physical_size);
                }
                WindowEvent::RedrawRequested => {
                    match renderer.render() {
                        Ok(_) => {}
                        Err(wgpu::SurfaceError::Lost) => renderer.resize(renderer.size),
                        Err(wgpu::SurfaceError::OutOfMemory) => target.exit(),
                        Err(e) => log::error!("Render error: {:?}", e),
                    }
                }
                _ => {}
            },
            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    })?;

    Ok(())
}
