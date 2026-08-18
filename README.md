# Quasar

**Quasar** is a lightweight, cross-platform GPU audio visualizer built in Rust. It captures system audio in real time, extracts frequency data via FFT, and drives custom GLSL/WGSL fragment shaders on a high-performance rendering surface.

Quasar enables instant live-reloading of custom shaders and color palettes without restarting the engine.

---

## Requirements

### Linux (Fedora / Ubuntu / Arch)
* **Vulkan Driver & Loader:** `vulkan-loader`, `mesa` / proprietary GPU drivers.
* **Audio Backend:** `pipewire` & `alsa-lib`.
* **Windowing:** `wayland`, `libxkbcommon`, `libX11`.
* **Build Tools:** `rustup` (Rust 1.75+) & `pkg-config`

### NixOS
A pre-configured `shell.nix` is included. You just need `nix-shell` or `direnv`.

### Windows
* **MSVC Build Tools:** Visual Studio 2022 C++ Build Tools or higher.
* **Rust Toolchain:** `stable-x86_64-pc-windows-msvc`.

---

## Getting Started

### General

```bash
# Clone the repository
git clone [https://github.com/your-username/quasar.git](https://github.com/your-username/quasar.git)
cd quasar

# Run the visualizer
cargo run --release
```


### NixOS specific
Before running:
```bash
# Enter the isolated environment
nix-shell --run "cargo run"
```

## Customization

Quasar stores its runtime shaders and color themes in your OS configuration directory:

* Linux: `~/.config/quasar/`
* Windows: `%APPDATA%\quasar\`

```
quasar/
├── shaders/
│   ├── accretion.glsl
│   └── singularity_bars.glsl
└── themes/
    ├── monochrome_void.json
    └── event_horizon.json
```

Example shaders and themes can be found in the [assets](assets/) directory.

### Writing custom shaders

Your GLSL shader needs the same bindings as the example shaders.

```glsl
#version 450

layout(location = 0) in vec2 fragCoord;
layout(location = 0) out vec4 fragColor;

layout(set = 0, binding = 0) uniform GlobalUniforms {
    vec2 u_resolution;      // Viewport width & height
    float u_time;           // Elapsed time in seconds
    float u_volume;         // Normalized global audio amplitude
    vec4 u_palette[4];      // RGBA colors loaded from active theme
};

layout(set = 0, binding = 1) uniform sampler1D u_audio_spectrum; // 512-bin FFT texture

void main() {
    float freq = texture(u_audio_spectrum, fragCoord.x / u_resolution.x).r;
    fragColor = vec4(u_palette[0].rgb * freq, 1.0);
}
```

### Custom colors

Look at the example theme files. Edit them to your liking.

The main settings are:

- `primary`
- `secondary`
- `background`
- `accent`