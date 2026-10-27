//! WebGL's extensions (M41 step 7, D40): those a context offers, by its
//! version and the adapter's features, and those the page enabled with
//! `getExtension`, which shaders and calls then take.

use std::collections::HashSet;

/// The adapter's features WebGL's extensions need, asked of the device
/// where the adapter has them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Features {
    pub bc: bool,
    pub float32_filterable: bool,
    pub float32_blendable: bool,
}

impl Features {
    /// From the adapter's features, by WebGPU's names.
    pub(crate) fn of(names: &[String]) -> Features {
        let has = |name: &str| names.iter().any(|n| n == name);
        Features {
            bc: has("texture-compression-bc"),
            float32_filterable: has("float32-filterable"),
            float32_blendable: has("float32-blendable"),
        }
    }

    /// The features a context's device is asked for.
    pub(crate) fn names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for (on, name) in [
            (self.bc, "texture-compression-bc"),
            (self.float32_filterable, "float32-filterable"),
            (self.float32_blendable, "float32-blendable"),
        ] {
            if on {
                names.push(name.to_owned());
            }
        }
        names
    }
}

/// The extensions a context offers, in the reference's order. WebGL 2's
/// own (EXT_color_buffer_float and its kin), OES_vertex_array_object,
/// WEBGL_draw_buffers and WEBGL_lose_context come with step 8.
pub(crate) fn offered(version: u8, features: &Features) -> Vec<&'static str> {
    let mut list = Vec::new();
    let mut add = |on: bool, name: &'static str| {
        if on {
            list.push(name);
        }
    };
    if version == 1 {
        add(true, "ANGLE_instanced_arrays");
        add(true, "EXT_blend_minmax");
        add(true, "EXT_color_buffer_half_float");
        add(features.float32_blendable, "EXT_float_blend");
        add(true, "EXT_frag_depth");
        add(true, "EXT_shader_texture_lod");
        add(true, "EXT_sRGB");
        add(true, "EXT_texture_filter_anisotropic");
        add(true, "OES_element_index_uint");
        add(true, "OES_fbo_render_mipmap");
        add(true, "OES_standard_derivatives");
        add(true, "OES_texture_float");
        add(features.float32_filterable, "OES_texture_float_linear");
        add(true, "OES_texture_half_float");
        add(true, "OES_texture_half_float_linear");
        add(true, "WEBGL_color_buffer_float");
        add(features.bc, "WEBGL_compressed_texture_s3tc");
        add(true, "WEBGL_depth_texture");
    } else {
        add(true, "EXT_texture_filter_anisotropic");
        add(features.float32_filterable, "OES_texture_float_linear");
        add(features.bc, "WEBGL_compressed_texture_s3tc");
    }
    list
}

/// The extensions a page enabled.
#[derive(Clone, Debug, Default)]
pub(crate) struct Enabled(HashSet<&'static str>);

impl Enabled {
    pub(crate) fn on(&self, name: &str) -> bool {
        self.0.contains(name)
    }

    /// `getExtension(name)`: the offered extension of that name, any
    /// case, enabled from now on; none for one not offered.
    pub(crate) fn enable(&mut self, offered: &[&'static str], name: &str) -> Option<&'static str> {
        let found = offered.iter().find(|o| o.eq_ignore_ascii_case(name))?;
        self.0.insert(found);
        // OES_texture_float brings the colour buffers it implies, as the
        // reference's does.
        if *found == "OES_texture_float" {
            self.0.insert("WEBGL_color_buffer_float");
        }
        if *found == "OES_texture_half_float" {
            self.0.insert("EXT_color_buffer_half_float");
        }
        Some(found)
    }
}
