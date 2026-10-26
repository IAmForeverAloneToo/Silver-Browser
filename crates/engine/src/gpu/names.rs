//! WebGPU's names for `wgpu`'s features and limits (M41 step 2, D39):
//! what a page is told an adapter or device has, and what it may ask
//! for.

/// The features a page may be told of, by WebGPU's names, with
/// `wgpu`'s for them. `core-features-and-limits` is every device's and
/// has none.
pub const FEATURES: [(&str, wgpu::Features); 16] = [
    ("depth-clip-control", wgpu::Features::DEPTH_CLIP_CONTROL),
    (
        "depth32float-stencil8",
        wgpu::Features::DEPTH32FLOAT_STENCIL8,
    ),
    (
        "texture-compression-bc",
        wgpu::Features::TEXTURE_COMPRESSION_BC,
    ),
    (
        "texture-compression-bc-sliced-3d",
        wgpu::Features::TEXTURE_COMPRESSION_BC_SLICED_3D,
    ),
    (
        "texture-compression-etc2",
        wgpu::Features::TEXTURE_COMPRESSION_ETC2,
    ),
    (
        "texture-compression-astc",
        wgpu::Features::TEXTURE_COMPRESSION_ASTC,
    ),
    (
        "texture-compression-astc-sliced-3d",
        wgpu::Features::TEXTURE_COMPRESSION_ASTC_SLICED_3D,
    ),
    ("timestamp-query", wgpu::Features::TIMESTAMP_QUERY),
    (
        "indirect-first-instance",
        wgpu::Features::INDIRECT_FIRST_INSTANCE,
    ),
    ("shader-f16", wgpu::Features::SHADER_F16),
    (
        "rg11b10ufloat-renderable",
        wgpu::Features::RG11B10UFLOAT_RENDERABLE,
    ),
    ("bgra8unorm-storage", wgpu::Features::BGRA8UNORM_STORAGE),
    ("float32-filterable", wgpu::Features::FLOAT32_FILTERABLE),
    ("float32-blendable", wgpu::Features::FLOAT32_BLENDABLE),
    ("dual-source-blending", wgpu::Features::DUAL_SOURCE_BLENDING),
    ("clip-distances", wgpu::Features::CLIP_DISTANCES),
];

/// Every device's feature, which names no `wgpu` feature.
pub const CORE: &str = "core-features-and-limits";

/// `features` by WebGPU's names, `core-features-and-limits` first.
pub fn feature_names(features: wgpu::Features) -> Vec<String> {
    let mut names = vec![CORE.to_owned()];
    for (name, feature) in FEATURES {
        if features.contains(feature) {
            names.push(name.to_owned());
        }
    }
    names
}

/// The features named; Err with a name that is none.
pub fn features_named(names: &[String]) -> Result<wgpu::Features, String> {
    let mut features = wgpu::Features::empty();
    for name in names {
        if name == CORE {
            continue;
        }
        match FEATURES.iter().find(|(known, _)| known == name) {
            Some((_, feature)) => features |= *feature,
            None => return Err(name.clone()),
        }
    }
    Ok(features)
}

macro_rules! limits {
    ($($name:literal => $field:ident),* $(,)?) => {
        /// WebGPU's limits, by their names, in the standard's order.
        pub const LIMITS: &[&str] = &[$($name),*];

        /// `limits` by WebGPU's names.
        pub fn limit_values(limits: &wgpu::Limits) -> Vec<(String, u64)> {
            vec![$(($name.to_owned(), u64::from(limits.$field))),*]
        }

        /// Sets limit `name` of `limits` to `value`: false for a name
        /// that is none, or a value its field cannot hold.
        pub fn set_limit(limits: &mut wgpu::Limits, name: &str, value: u64) -> bool {
            match name {
                $($name => match value.try_into() {
                    Ok(value) => {
                        limits.$field = value;
                        true
                    }
                    Err(_) => false,
                },)*
                _ => false,
            }
        }
    };
}

limits! {
    "maxTextureDimension1D" => max_texture_dimension_1d,
    "maxTextureDimension2D" => max_texture_dimension_2d,
    "maxTextureDimension3D" => max_texture_dimension_3d,
    "maxTextureArrayLayers" => max_texture_array_layers,
    "maxBindGroups" => max_bind_groups,
    "maxBindGroupsPlusVertexBuffers" => max_bind_groups_plus_vertex_buffers,
    "maxBindingsPerBindGroup" => max_bindings_per_bind_group,
    "maxDynamicUniformBuffersPerPipelineLayout" => max_dynamic_uniform_buffers_per_pipeline_layout,
    "maxDynamicStorageBuffersPerPipelineLayout" => max_dynamic_storage_buffers_per_pipeline_layout,
    "maxSampledTexturesPerShaderStage" => max_sampled_textures_per_shader_stage,
    "maxSamplersPerShaderStage" => max_samplers_per_shader_stage,
    "maxStorageBuffersPerShaderStage" => max_storage_buffers_per_shader_stage,
    "maxStorageTexturesPerShaderStage" => max_storage_textures_per_shader_stage,
    "maxUniformBuffersPerShaderStage" => max_uniform_buffers_per_shader_stage,
    "maxUniformBufferBindingSize" => max_uniform_buffer_binding_size,
    "maxStorageBufferBindingSize" => max_storage_buffer_binding_size,
    "minUniformBufferOffsetAlignment" => min_uniform_buffer_offset_alignment,
    "minStorageBufferOffsetAlignment" => min_storage_buffer_offset_alignment,
    "maxVertexBuffers" => max_vertex_buffers,
    "maxBufferSize" => max_buffer_size,
    "maxVertexAttributes" => max_vertex_attributes,
    "maxVertexBufferArrayStride" => max_vertex_buffer_array_stride,
    "maxInterStageShaderVariables" => max_inter_stage_shader_variables,
    "maxColorAttachments" => max_color_attachments,
    "maxColorAttachmentBytesPerSample" => max_color_attachment_bytes_per_sample,
    "maxComputeWorkgroupStorageSize" => max_compute_workgroup_storage_size,
    "maxComputeInvocationsPerWorkgroup" => max_compute_invocations_per_workgroup,
    "maxComputeWorkgroupSizeX" => max_compute_workgroup_size_x,
    "maxComputeWorkgroupSizeY" => max_compute_workgroup_size_y,
    "maxComputeWorkgroupSizeZ" => max_compute_workgroup_size_z,
    "maxComputeWorkgroupsPerDimension" => max_compute_workgroups_per_dimension,
}
