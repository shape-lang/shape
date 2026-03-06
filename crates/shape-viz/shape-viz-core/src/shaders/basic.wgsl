// Basic vertex and fragment shaders for FChart rendering

struct ScreenUniforms {
    width: f32,
    height: f32,
}

@group(0) @binding(0)
var<uniform> screen: ScreenUniforms;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

// Vertex shader
@vertex
fn vs_main(
    model: VertexInput,
    @builtin(vertex_index) vertex_index: u32,
) -> VertexOutput {
    var out: VertexOutput;

    // Convert from screen coordinates to NDC
    // Screen: (0,0) at top-left, (width,height) at bottom-right
    // NDC: (-1,1) at top-left, (1,-1) at bottom-right (Y is flipped)

    let ndc_x = (model.position.x / screen.width) * 2.0 - 1.0;
    let ndc_y = 1.0 - (model.position.y / screen.height) * 2.0;

    out.clip_position = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    out.color = model.color;

    return out;
}

// Fragment shader
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}