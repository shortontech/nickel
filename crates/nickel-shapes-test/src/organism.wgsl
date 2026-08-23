struct Scene {
    angle: f32,
    aspect: f32,
    padding: vec2<f32>,
};

@group(0) @binding(0) var<uniform> scene: Scene;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) surface_feature: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) view_position: vec3<f32>,
    @location(3) surface_feature: vec3<f32>,
};

fn rotate_y(value: vec3<f32>, angle: f32) -> vec3<f32> {
    let sine = sin(angle);
    let cosine = cos(angle);
    return vec3<f32>(
        value.x * cosine + value.z * sine,
        value.y,
        -value.x * sine + value.z * cosine,
    );
}

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    let world = rotate_y(input.position, scene.angle);
    let view = world + vec3<f32>(0.0, -0.05, 3.2);
    let focal = 2.5;
    var output: VertexOutput;
    output.position = vec4<f32>(
        view.x * focal / scene.aspect,
        view.y * focal,
        view.z - 0.1,
        view.z,
    );
    output.normal = normalize(rotate_y(input.normal, scene.angle));
    output.color = input.color;
    output.view_position = view;
    output.surface_feature = input.surface_feature;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(input.normal);
    let view_direction = normalize(-input.view_position);
    let key = normalize(vec3<f32>(-0.55, 0.78, -0.45));
    let fill = normalize(vec3<f32>(0.65, 0.20, -0.30));
    let key_light = max(dot(normal, key), 0.0);
    let fill_light = max(dot(normal, fill), 0.0) * 0.28;
    let half_vector = normalize(key + view_direction);
    let specular = pow(max(dot(normal, half_vector), 0.0), 38.0) * 0.32;
    let rim = pow(1.0 - max(dot(normal, view_direction), 0.0), 3.0) * 0.22;
    let subsurface = pow(max(dot(-normal, key), 0.0), 2.0) * input.color * 0.12;
    let aperture_shape = dot(input.surface_feature.xy, input.surface_feature.xy);
    let aperture = smoothstep(1.0, 0.72, aperture_shape)
        * (1.0 - smoothstep(0.82, 1.0, input.surface_feature.z));
    let surface_color = mix(input.color, vec3<f32>(0.055, 0.026, 0.022), aperture);
    let lit = surface_color * (0.22 + key_light * 0.72 + fill_light)
        + subsurface * (1.0 - aperture);
    return vec4<f32>(lit + vec3<f32>(specular + rim), 1.0);
}
