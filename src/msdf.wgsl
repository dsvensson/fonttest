struct Globals {
    viewport_atlas: vec4<f32>,
    range_padding: vec4<f32>,
};

@group(0) @binding(0) var msdf_atlas: texture_2d<f32>;
@group(0) @binding(1) var msdf_sampler: sampler;
@group(0) @binding(2) var<uniform> globals: Globals;

struct VertexInput {
    @location(0) rect: vec4<f32>,
    @location(1) uv_rect: vec4<f32>,
    @location(2) fill_top: vec4<f32>,
    @location(3) fill_bottom: vec4<f32>,
    @location(4) outline_color: vec4<f32>,
    @location(5) shadow_color: vec4<f32>,
    @location(6) effect_params: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fill_color: vec4<f32>,
    @location(2) outline_color: vec4<f32>,
    @location(3) shadow_color: vec4<f32>,
    @location(4) effect_params: vec4<f32>,
};

fn quad_corner(vertex_index: u32) -> vec2<f32> {
    let corners = array(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    return corners[vertex_index];
}

fn vertex_common(input: VertexInput, vertex_index: u32, shadow: bool) -> VertexOutput {
    let corner = quad_corner(vertex_index);
    var screen_position = input.rect.xy + corner * input.rect.zw;
    if shadow {
        screen_position += input.effect_params.zw;
    }
    let viewport = globals.viewport_atlas.xy;
    let clip = vec2<f32>(
        screen_position.x / viewport.x * 2.0 - 1.0,
        1.0 - screen_position.y / viewport.y * 2.0,
    );

    var output: VertexOutput;
    output.clip_position = vec4<f32>(clip, 0.0, 1.0);
    output.uv = mix(input.uv_rect.xy, input.uv_rect.zw, corner);
    output.fill_color = mix(input.fill_top, input.fill_bottom, corner.y);
    output.outline_color = input.outline_color;
    output.shadow_color = input.shadow_color;
    output.effect_params = input.effect_params;
    return output;
}

@vertex
fn vs_main(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    return vertex_common(input, vertex_index, false);
}

@vertex
fn vs_shadow(input: VertexInput, @builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    return vertex_common(input, vertex_index, true);
}

fn median3(value: vec3<f32>) -> f32 {
    return max(min(value.r, value.g), min(max(value.r, value.g), value.b));
}

fn signed_distance_pixels(uv: vec2<f32>) -> f32 {
    let encoded_distance = median3(textureSample(msdf_atlas, msdf_sampler, uv).rgb) - 0.5;
    let unit_range = vec2<f32>(globals.range_padding.x) / globals.viewport_atlas.zw;
    let screen_texture_size = 1.0 / max(fwidth(uv), vec2<f32>(0.000001));
    let screen_range = max(0.5 * dot(unit_range, screen_texture_size), 1.0);
    return encoded_distance * screen_range;
}

@fragment
fn fs_shadow(input: VertexOutput) -> @location(0) vec4<f32> {
    let distance = signed_distance_pixels(input.uv);
    let softness = max(input.effect_params.y, 0.75);
    let coverage = smoothstep(-softness, 0.5, distance);
    let alpha = input.shadow_color.a * coverage;
    if alpha < 0.001 {
        discard;
    }
    return vec4<f32>(input.shadow_color.rgb * alpha, alpha);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let distance = signed_distance_pixels(input.uv);
    let fill_coverage = clamp(distance + 0.5, 0.0, 1.0);
    let outline_coverage = clamp(distance + input.effect_params.x + 0.5, 0.0, 1.0);
    let fill_alpha = input.fill_color.a * fill_coverage;
    let outline_alpha = input.outline_color.a * outline_coverage;
    let under = outline_alpha * (1.0 - fill_alpha);
    let alpha = fill_alpha + under;
    if alpha < 0.001 {
        discard;
    }
    let color = input.fill_color.rgb * fill_alpha + input.outline_color.rgb * under;
    return vec4<f32>(color, alpha);
}

