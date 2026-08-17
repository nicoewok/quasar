#version 450
layout(location = 0) out vec4 fragColor;

layout(set = 0, binding = 0) uniform Uniforms {
    vec2 u_resolution;
    float u_time;
    float u_volume;
    vec4 u_palette[4];
};

layout(set = 0, binding = 1) uniform texture2D u_spectrum;
layout(set = 0, binding = 2) uniform sampler u_sampler;

// Helper function to sample normalized audio spectrum given frequency u [0.0, 1.0]
float sample_spectrum(float u) {
    return texture(sampler2D(u_spectrum, u_sampler), vec2(clamp(u, 0.0, 1.0), 0.5)).r;
}

void main() {
    // Center coordinates with aspect ratio correction
    vec2 st = (gl_FragCoord.xy - 0.5 * u_resolution) / min(u_resolution.x, u_resolution.y);
    
    float r = length(st);
    float angle = atan(st.y, st.x);
    float norm_angle = (angle + 3.14159265359) / (2.0 * 3.14159265359);
    
    // Multi-band audio spectrum sampling
    float audio_bass = sample_spectrum(0.05);
    float audio_mid = sample_spectrum(0.3);
    float audio_high = sample_spectrum(0.7);
    float audio_angle = sample_spectrum(norm_angle);
    
    // Event horizon radius (pulsating with bass and global volume)
    float horizon_r = 0.18 + 0.06 * audio_bass * (0.5 + u_volume);
    
    // Accretion disk radius and spiral distortion
    float spiral = angle * 4.0 + u_time * 2.5;
    float ring_distort = 0.03 * sin(spiral + audio_angle * 6.28) + 0.05 * audio_angle;
    float accretion_inner = horizon_r + 0.015;
    float accretion_outer = horizon_r + 0.16 + 0.14 * (audio_mid + u_volume) + ring_distort;
    
    // Relativistic polar light jets (along vertical y-axis)
    float jet_x = abs(st.x) - (0.015 + 0.025 * audio_high);
    float jet_intensity = smoothstep(0.06 + 0.04 * audio_high, 0.0, jet_x) * smoothstep(0.12, 0.45, abs(st.y));
    jet_intensity *= (0.6 + 0.4 * sin(u_time * 12.0 + st.y * 25.0)) * (audio_high + u_volume);
    
    // Photonic ring around the event horizon
    float photon_ring = step(abs(r - (horizon_r + 0.008)), 0.006 + 0.004 * audio_high);
    
    // Accretion disk intensity field
    float is_accretion = step(accretion_inner, r) * step(r, accretion_outer);
    float disk_pattern = is_accretion * (0.7 + 0.4 * sin(spiral * 3.0 + r * 50.0));
    
    // Central void (event horizon) mask
    float is_void = step(r, horizon_r);
    
    // Combine fields for high-contrast 1-bit rendering
    float total_intensity = disk_pattern + photon_ring + jet_intensity;
    
    // Binary thresholding for crisp 1-bit aesthetics
    float bit_signal = step(0.35, total_intensity) * (1.0 - is_void);
    
    // Map binary signal to palette (Void Black vs Pure White / Off-White)
    vec4 final_color = mix(u_palette[2], u_palette[0], bit_signal);
    
    fragColor = final_color;
}
