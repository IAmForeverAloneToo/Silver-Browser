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
    pub depth32_stencil: bool,
    pub rg11b10_renderable: bool,
}

impl Features {
    /// From the adapter's features, by WebGPU's names.
    pub(crate) fn of(names: &[String]) -> Features {
        let has = |name: &str| names.iter().any(|n| n == name);
        Features {
            bc: has("texture-compression-bc"),
            float32_filterable: has("float32-filterable"),
            float32_blendable: has("float32-blendable"),
            depth32_stencil: has("depth32float-stencil8"),
            rg11b10_renderable: has("rg11b10ufloat-renderable"),
        }
    }

    /// The features a context's device is asked for.
    pub(crate) fn names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for (on, name) in [
            (self.bc, "texture-compression-bc"),
            (self.float32_filterable, "float32-filterable"),
            (self.float32_blendable, "float32-blendable"),
            (self.depth32_stencil, "depth32float-stencil8"),
            (self.rg11b10_renderable, "rg11b10ufloat-renderable"),
        ] {
            if on {
                names.push(name.to_owned());
            }
        }
        names
    }
}

/// The extensions a context offers, in the reference's order.
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
        add(true, "OES_vertex_array_object");
        add(true, "WEBGL_color_buffer_float");
        add(features.bc, "WEBGL_compressed_texture_s3tc");
        add(true, "WEBGL_debug_renderer_info");
        add(true, "WEBGL_depth_texture");
        add(true, "WEBGL_draw_buffers");
        add(true, "WEBGL_lose_context");
    } else {
        add(true, "EXT_color_buffer_float");
        add(true, "EXT_color_buffer_half_float");
        add(features.float32_blendable, "EXT_float_blend");
        add(true, "EXT_texture_filter_anisotropic");
        add(features.float32_filterable, "OES_texture_float_linear");
        add(features.bc, "WEBGL_compressed_texture_s3tc");
        add(true, "WEBGL_debug_renderer_info");
        add(true, "WEBGL_lose_context");
    }
    list
}

/// The adapter a context draws with, as `WEBGL_debug_renderer_info`
/// names it (2026-11-20): its PCI vendor and device, its description,
/// and whether it is the software fallback.
#[derive(Clone, Debug, Default)]
pub(crate) struct Adapter {
    pub vendor: u32,
    pub device: u32,
    pub description: String,
    pub software: bool,
}

/// The longest description kept, in characters.
const MAX_DESCRIPTION: usize = 256;

/// SwiftShader's renderer string, the reference's where it has no GPU.
const SWIFTSHADER: &str =
    "ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver)";

impl Adapter {
    /// From what the GPU process told of it: its description cut to
    /// [`MAX_DESCRIPTION`], without control characters.
    pub(crate) fn of(info: &crate::gpu::wire::AdapterInfo) -> Adapter {
        Adapter {
            vendor: info.pci_vendor,
            device: info.pci_device,
            description: info
                .description
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_DESCRIPTION)
                .collect(),
            software: info.fallback,
        }
    }

    /// The vendor as ANGLE names it in the reference's strings.
    fn short(&self) -> &'static str {
        match self.vendor {
            0x8086 => "Intel",
            0x10de => "NVIDIA",
            0x1002 | 0x1022 => "AMD",
            0x1414 => "Microsoft",
            0x5143 => "Qualcomm",
            0x13b5 => "ARM",
            0x106b => "Apple",
            0x1ae0 => "Google",
            _ => "Unknown",
        }
    }

    /// `UNMASKED_VENDOR_WEBGL`, as the reference gives it (probed, Chrome
    /// 156 on Windows: "Google Inc. (Intel)").
    pub(crate) fn unmasked_vendor(&self) -> String {
        if self.software {
            return "Google Inc. (Google)".to_owned();
        }
        format!("Google Inc. ({})", self.short())
    }

    /// `UNMASKED_RENDERER_WEBGL`, as the reference gives it for the
    /// adapter on this platform (probed, Chrome 156 on Windows: "ANGLE
    /// (Intel, Intel(R) UHD Graphics (0x00009A60) Direct3D11 vs_5_0
    /// ps_5_0, D3D11)"); SwiftShader's for the software fallback, which
    /// the reference draws WebGL with where it has no GPU.
    pub(crate) fn unmasked_renderer(&self) -> String {
        if self.software {
            return SWIFTSHADER.to_owned();
        }
        let (short, description) = (self.short(), &self.description);
        if cfg!(target_os = "windows") {
            format!(
                "ANGLE ({short}, {description} (0x{:08X}) Direct3D11 vs_5_0 ps_5_0, D3D11)",
                self.device
            )
        } else if cfg!(target_os = "macos") {
            format!("ANGLE (Apple, ANGLE Metal Renderer: {description}, Unspecified Version)")
        } else {
            format!("ANGLE ({short}, {description}, OpenGL 4.6)")
        }
    }
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

#[cfg(test)]
mod tests {
    use super::Adapter;

    /// The reference's strings for the adapter it was probed on (Chrome
    /// 156, Windows, 2026-11-20), and SwiftShader's for the fallback.
    #[test]
    fn the_debug_renderer_info_strings_are_the_references() {
        let intel = Adapter {
            vendor: 0x8086,
            device: 0x9a60,
            description: "Intel(R) UHD Graphics".to_owned(),
            software: false,
        };
        assert_eq!(intel.unmasked_vendor(), "Google Inc. (Intel)");
        if cfg!(target_os = "windows") {
            assert_eq!(
                intel.unmasked_renderer(),
                "ANGLE (Intel, Intel(R) UHD Graphics (0x00009A60) Direct3D11 vs_5_0 ps_5_0, D3D11)"
            );
        }
        let software = Adapter {
            software: true,
            ..intel
        };
        assert_eq!(software.unmasked_vendor(), "Google Inc. (Google)");
        assert!(software.unmasked_renderer().contains("SwiftShader"));
    }
}
