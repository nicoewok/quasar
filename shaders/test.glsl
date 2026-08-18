#version 450

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 fragColor;

layout(set = 0, binding = 0) uniform Globals {
    float u_time;
    float u_padding;
    vec2 u_resolution;
};

layout(set = 0, binding = 1) uniform texture2D u_audio;
layout(set = 0, binding = 2) uniform sampler u_audio_sampler;

void main() {
    vec2 uv = v_uv;

    // Read audio frequency spectrum data at uv.x
    float freq = texture(sampler2D(u_audio, u_audio_sampler), vec2(uv.x, 0.5)).r;

    // Centered aspect-ratio normalized coordinates
    vec2 st = (gl_FragCoord.xy - 0.5 * u_resolution) / u_resolution.y;

    // Distort coordinates based on audio frequency intensity
    st += vec2(sin(st.y * 10.0 + u_time), cos(st.x * 10.0 + u_time)) * freq * 0.15;

    // Base color combined with frequency reactive bass/treble
    vec3 col = vec3(uv.x + freq * 0.5, uv.y, 0.5 + 0.5 * sin(u_time + freq * 3.0));

    // Dynamic radial ripples amplified by audio
    float dist = length(st);
    float ring = sin(dist * (25.0 + freq * 30.0) - u_time * 4.0);
    col += vec3(0.2, 0.5 * freq, 0.8) * ring * (0.5 + freq);

    // Audio bar visualizer at bottom of screen
    if (uv.y < 0.15) {
        float bar = step(uv.y, freq * 0.15);
        col = mix(col, vec3(0.0, 1.0, 0.8), bar);
    }

    // Grid lines for UV test pattern distorted by audio
    vec2 grid = abs(fract(uv * 10.0 - 0.5) - 0.5) / fwidth(uv * 10.0);
    float grid_line = min(grid.x, grid.y);
    float grid_val = 1.0 - min(grid_line, 1.0);
    col = mix(col, vec3(1.0), grid_val * 0.4 * (1.0 + freq));

    // Vignette border
    float vignette = uv.x * uv.y * (1.0 - uv.x) * (1.0 - uv.y);
    vignette = clamp(pow(16.0 * vignette, 0.25), 0.0, 1.0);
    col *= vignette;

    fragColor = vec4(col, 1.0);
}
