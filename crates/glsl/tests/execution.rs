//! The translation run (M41 step 5): each program drawn on this
//! machine's adapter into a pixel of floats, its value read back, so
//! that what the WGSL means is GLSL's, not only valid. Where there is no
//! adapter (CI's Windows runners), a test says so and checks nothing.

#![forbid(unsafe_code)]

use glsl::{LinkOptions, Options, Program, Stage, compile, link};

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Option<Gpu> {
    let ours = wgpu::Backends::VULKAN | wgpu::Backends::METAL | wgpu::Backends::DX12;
    let backends = wgpu::Backends::from_env()
        .map(|named| named & ours)
        .filter(|named| !named.is_empty())
        .unwrap_or(ours);
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        flags: wgpu::InstanceFlags::empty(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some(Gpu { device, queue })
}

const VS1: &str =
    "attribute vec2 a_position;\nvoid main() { gl_Position = vec4(a_position, 0.0, 1.0); }\n";
const VS3: &str = "#version 300 es\nin vec2 a_position;\nvoid main() { gl_Position = vec4(a_position, 0.0, 1.0); }\n";

/// The value a fragment shader writes, with `uniforms` set by their
/// names (each a list of floats, or of u32s for ints and bools).
fn run(gpu: &Gpu, vs: &str, fs: &str, webgl2: bool, uniforms: &[(&str, Vec<u32>)]) -> [f32; 4] {
    let options = Options {
        webgl2,
        ..Options::default()
    };
    let v = compile(vs, Stage::Vertex, &options).unwrap_or_else(|log| panic!("vertex: {log}"));
    let f = compile(fs, Stage::Fragment, &options)
        .unwrap_or_else(|log| panic!("fragment: {log}\n{fs}"));
    let program: Program =
        link(&v, &f, &LinkOptions::default()).unwrap_or_else(|log| panic!("link: {log}"));
    let device = &gpu.device;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(program.vertex.clone().into()),
    });
    let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(program.fragment.clone().into()),
    });
    // The default uniforms and the blocks', as the reflection places
    // them.
    let mut bytes = vec![0u8; program.uniform_size.max(16) as usize];
    let mut blocks: Vec<Vec<u8>> = program
        .blocks
        .iter()
        .map(|b| vec![0u8; b.size as usize])
        .collect();
    for (name, values) in uniforms {
        let uniform = program
            .uniforms
            .iter()
            .find(|u| u.name == *name)
            .unwrap_or_else(|| panic!("{name}: {:?}", program.uniforms));
        // Each element's components in turn; a matrix's columns at its
        // stride, of rows components each.
        let (rows, cols) = match uniform.gl_type {
            0x8B5A => (2, 2),
            0x8B5B => (3, 3),
            0x8B5C => (4, 4),
            0x1406 | 0x1404 | 0x1405 | 0x8B56 => (1, 1),
            0x8B50 | 0x8B53 | 0x8B57 | 0x8DC6 => (2, 1),
            0x8B51 | 0x8B54 | 0x8B58 | 0x8DC7 => (3, 1),
            _ => (4, 1),
        };
        let per = rows * cols;
        for (i, chunk) in values.chunks(per).enumerate() {
            let base = uniform.offset as usize + i * uniform.array_stride as usize;
            for (k, value) in chunk.iter().enumerate() {
                let (col, row) = (k / rows, k % rows);
                let at = base + col * uniform.matrix_stride as usize + row * 4;
                let target = match uniform.block {
                    Some(b) => &mut blocks[b],
                    None => &mut bytes,
                };
                target[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        }
    }
    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue.write_buffer(&uniform_buffer, 0, &bytes);
    let block_buffers: Vec<(u32, wgpu::Buffer)> = program
        .blocks
        .iter()
        .zip(&blocks)
        .map(|(block, data)| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: data.len() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            gpu.queue.write_buffer(&buffer, 0, data);
            (block.bindings[0], buffer)
        })
        .collect();
    let buffer_entry = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let mut layout_entries = vec![buffer_entry(0)];
    let mut group_entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniform_buffer.as_entire_binding(),
    }];
    for (binding, buffer) in &block_buffers {
        layout_entries.push(buffer_entry(*binding));
        group_entries.push(wgpu::BindGroupEntry {
            binding: *binding,
            resource: buffer.as_entire_binding(),
        });
    }
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &layout_entries,
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &layout,
        entries: &group_entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let attributes = [wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: program.attributes[0].location,
    }];
    let targets: Vec<Option<wgpu::ColorTargetState>> = (0..program.outputs.len().max(1))
        .map(|_| {
            Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba32Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect();
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &vertex,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: 8,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &fragment,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
    });
    let positions: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
    let vertices = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 24,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let raw: Vec<u8> = positions.iter().flat_map(|p| p.to_le_bytes()).collect();
    gpu.queue.write_buffer(&vertices, 0, &raw);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let extra: Vec<wgpu::Texture> = (1..targets.len())
        .map(|_| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        })
        .collect();
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let extra_views: Vec<wgpu::TextureView> = extra
        .iter()
        .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()))
        .collect();
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut attachments = vec![Some(wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })];
        for v in &extra_views {
            attachments.push(Some(wgpu::RenderPassColorAttachment {
                view: v,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            }));
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    if let Some(error) = pollster::block_on(scope.pop()) {
        panic!("{error}\n{}\n{}", program.vertex, program.fragment);
    }
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = read.slice(..16).get_mapped_range().expect("mapped");
    let mut out = [0.0f32; 4];
    for (i, v) in out.iter_mut().enumerate() {
        *v = f32::from_le_bytes([
            data[i * 4],
            data[i * 4 + 1],
            data[i * 4 + 2],
            data[i * 4 + 3],
        ]);
    }
    out
}

fn fs1(body: &str) -> String {
    format!("precision highp float;\n{body}\n")
}

fn f(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}

#[test]
fn glsl_es_1_00_means_what_it_says() {
    let Some(gpu) = gpu() else {
        eprintln!("no adapter here: the translation is not run");
        return;
    };
    let cases: Vec<(&str, [f32; 4])> = vec![
        // A swizzle's stores, and compound ones.
        (
            "void main() { vec4 v = vec4(1.0, 2.0, 3.0, 4.0); v.zx = vec2(10.0, 20.0); v.yw += vec2(1.0); gl_FragColor = v; }",
            [20.0, 3.0, 10.0, 5.0],
        ),
        // Side effects in GLSL's order.
        (
            "void main() { float c = 1.0; float r = c++ * 10.0 + c; float d = (c += 1.0) * c; gl_FragColor = vec4(r, c, d, 0.0); }",
            [12.0, 3.0, 9.0, 0.0],
        ),
        // An operand read before a later one's assignment changes it.
        (
            "void main() { float c = 1.0; float r = c + (c = 5.0); gl_FragColor = vec4(r, c, 0.0, 0.0); }",
            [6.0, 5.0, 0.0, 0.0],
        ),
        // Short circuits and the conditional.
        (
            "void main() { float n = 0.0; bool b = false && (n++ > 0.0); bool t = true || (n++ > 0.0); float e = b ? (n += 5.0) : (n += 1.0); gl_FragColor = vec4(n, e, float(t), float(b)); }",
            [1.0, 1.0, 1.0, 0.0],
        ),
        // Out and inout parameters.
        (
            "void f(inout float x, out float y) { x *= 2.0; y = x + 1.0; }\nvoid main() { float x = 3.0; float y = 0.0; f(x, y); vec2 v = vec2(1.0); f(v.y, v.x); gl_FragColor = vec4(x, y, v); }",
            [6.0, 7.0, 3.0, 2.0],
        ),
        // mod's sign (of a variable: a constant's is folded), and the
        // matrices' products, column-major.
        (
            "void main() { float n = -1.0; mat2 m = mat2(1.0, 2.0, 3.0, 4.0); vec2 a = m * vec2(1.0); vec2 b = vec2(1.0) * m; gl_FragColor = vec4(mod(n, 3.0), a.y, b.y, m[1][0]); }",
            [2.0, 6.0, 7.0, 3.0],
        ),
        // A matrix from a matrix, the identity's rest; matrixCompMult.
        (
            "void main() { mat3 m = mat3(mat2(2.0)); mat2 c = matrixCompMult(mat2(1.0, 2.0, 3.0, 4.0), mat2(2.0)); gl_FragColor = vec4(m[0][0], m[2][2], m[1][2], c[1][1]); }",
            [2.0, 1.0, 0.0, 8.0],
        ),
        // A loop's continue and break.
        (
            "void main() { float s = 0.0; for (int i = 0; i < 10; i++) { if (i == 2) continue; if (i == 5) break; s += float(i); } gl_FragColor = vec4(s); }",
            [8.0, 8.0, 8.0, 8.0],
        ),
        // A struct's equality, and constants folded.
        (
            "struct P { vec2 a; float b; };\nconst float k = 2.0 * 3.0 + 1.0;\nvoid main() { P x = P(vec2(1.0), k); P y = x; y.b += 1.0; gl_FragColor = vec4(float(x == x), float(x == y), k, float(vec2(1.0) == vec2(1.0, 2.0))); }",
            [1.0, 0.0, 7.0, 0.0],
        ),
    ];
    for (body, want) in cases {
        let got = run(&gpu, VS1, &fs1(body), false, &[]);
        assert_eq!(got, want, "{body}");
    }
}

#[test]
fn uniforms_are_read_where_the_reflection_says_they_are() {
    let Some(gpu) = gpu() else {
        eprintln!("no adapter here: the translation is not run");
        return;
    };
    let fs = fs1(
        "uniform float u_list[3];\nuniform vec3 u_v;\nuniform bool u_b;\nuniform mat2 u_m;\nuniform ivec2 u_i;\nstruct S { vec2 a; float b; };\nuniform S u_s[2];\n\
         void main() { gl_FragColor = vec4(u_list[0] + u_list[2], u_v.y + float(u_b), u_m[1][0] + float(u_i.y), u_s[1].a.y + u_s[1].b + u_s[0].b); }",
    );
    let got = run(
        &gpu,
        VS1,
        &fs,
        false,
        &[
            ("u_list[0]", f(&[1.0, 2.0, 4.0])),
            ("u_v", f(&[0.0, 10.0, 0.0])),
            ("u_b", vec![1]),
            ("u_m", f(&[0.0, 0.0, 20.0, 0.0])),
            ("u_i", vec![0, 3]),
            ("u_s[0].b", f(&[100.0])),
            ("u_s[1].a", f(&[0.0, 30.0])),
            ("u_s[1].b", f(&[5.0])),
        ],
    );
    assert_eq!(got, [5.0, 11.0, 23.0, 135.0]);
}

#[test]
fn glsl_es_3_00_means_what_it_says() {
    let Some(gpu) = gpu() else {
        eprintln!("no adapter here: the translation is not run");
        return;
    };
    let fs =
        |body: &str| format!("#version 300 es\nprecision highp float;\nout vec4 color;\n{body}\n");
    let cases: Vec<(String, [f32; 4])> = vec![
        // A switch's fallthrough, its default, and a case that breaks.
        (
            fs(
                "int pick(int s) { int n = 0; switch (s) { case 5: n = 1; case 6: n += 2; break; default: n = 7; } return n; }\nvoid main() { color = vec4(float(pick(5)), float(pick(6)), float(pick(9)), 0.0); }",
            ),
            [3.0, 2.0, 7.0, 0.0],
        ),
        // Integers: bits, shifts, remainders, uint's wrap.
        (
            fs(
                "void main() { int a = 13; uint u = 0u; u -= 1u; color = vec4(float(a & 6), float(a << 2), float(a % 5), float(u >> 28u)); }",
            ),
            [4.0, 52.0, 3.0, 15.0],
        ),
        // A do-while, a while with continue, arrays and their equality.
        (
            fs(
                "void main() { int i = 0; do { i++; } while (i < 3); int s = 0; while (i > 0) { i--; if (i == 1) continue; s += i; } float a[2] = float[2](1.0, 2.0); float b[2] = a; b[1] = 3.0; color = vec4(float(s), float(a == a), float(a == b), float(a.length())); }",
            ),
            [2.0, 1.0, 0.0, 2.0],
        ),
    ];
    for (body, want) in cases {
        let got = run(&gpu, VS3, &body, true, &[]);
        assert_eq!(got, want, "{body}");
    }
    // Uniform blocks in std140, written where the reflection says, a
    // row-major matrix among them.
    let block = fs(
        "layout(std140) uniform Data { vec3 v; float f; mat2 m; float list[2]; layout(row_major) mat2 r; } data;\nvoid main() { color = vec4(data.v.z + data.f, data.m[1][0], data.list[1], data.r[1][0]); }",
    );
    let got = run(
        &gpu,
        VS3,
        &block,
        true,
        &[
            ("Data.v", f(&[0.0, 0.0, 5.0])),
            ("Data.f", f(&[2.0])),
            ("Data.m", f(&[0.0, 0.0, 3.0, 0.0])),
            ("Data.list[0]", f(&[0.0, 4.0])),
            // Row-major: the rows in turn; column 1 row 0 is the first
            // row's second.
            ("Data.r", f(&[0.0, 6.0, 0.0, 0.0])),
        ],
    );
    assert_eq!(got, [7.0, 3.0, 4.0, 6.0]);
}
