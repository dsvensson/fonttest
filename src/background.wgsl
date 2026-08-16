struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let positions = array(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    let position = positions[vertex_index];
    var output: VertexOutput;
    output.clip_position = vec4<f32>(position, 0.0, 1.0);
    output.uv = position * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let top = vec3<f32>(0.028, 0.039, 0.082);
    let bottom = vec3<f32>(0.006, 0.010, 0.026);
    var color = mix(top, bottom, smoothstep(0.0, 1.0, input.uv.y));

    let glow_delta = input.uv - vec2<f32>(0.28, 0.16);
    let glow = exp(-dot(glow_delta, glow_delta) * 7.0);
    color += vec3<f32>(0.035, 0.055, 0.13) * glow;

    let vignette_delta = input.uv - vec2<f32>(0.5);
    let vignette = smoothstep(0.22, 0.78, dot(vignette_delta, vignette_delta));
    color *= 1.0 - 0.52 * vignette;
    return vec4<f32>(color, 1.0);
}

