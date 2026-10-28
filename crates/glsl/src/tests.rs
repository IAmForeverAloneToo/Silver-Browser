//! The crate's shaders: what compiles and what does not, in the
//! reference's words; programs that link, their WGSL validated by naga;
//! their attributes, uniforms and blocks where they are.

use crate::{LinkOptions, Options, Program, Stage, compile, link};

fn webgl1() -> Options {
    Options::default()
}

fn webgl2() -> Options {
    Options {
        webgl2: true,
        ..Options::default()
    }
}

/// The info log of a shader that does not compile.
fn error(source: &str, stage: Stage, options: &Options) -> String {
    match compile(source, stage, options) {
        Ok(_) => panic!("compiled: {source}"),
        Err(log) => log,
    }
}

/// The program of two shaders that compile and link.
fn program(vs: &str, fs: &str, options: &Options) -> Program {
    let v = compile(vs, Stage::Vertex, options).unwrap_or_else(|log| panic!("vertex: {log}"));
    let f = compile(fs, Stage::Fragment, options).unwrap_or_else(|log| panic!("fragment: {log}"));
    link(&v, &f, &LinkOptions::default()).unwrap_or_else(|log| panic!("link: {log}"))
}

const TRIANGLE_VS: &str = "attribute vec4 a_position;\n\
    attribute vec2 a_uv;\n\
    uniform mat4 u_matrix;\n\
    varying vec2 v_uv;\n\
    void main() {\n\
      v_uv = a_uv;\n\
      gl_Position = u_matrix * a_position;\n\
    }\n";

const TRIANGLE_FS: &str = "precision mediump float;\n\
    uniform sampler2D u_texture;\n\
    uniform vec4 u_tint;\n\
    varying vec2 v_uv;\n\
    void main() {\n\
      gl_FragColor = texture2D(u_texture, v_uv) * u_tint;\n\
    }\n";

#[test]
fn a_textured_triangle_links_with_its_attributes_uniforms_and_sampler() {
    let p = program(TRIANGLE_VS, TRIANGLE_FS, &webgl1());
    let names: Vec<(&str, u32, u32)> = p
        .attributes
        .iter()
        .map(|a| (a.name.as_str(), a.location, a.gl_type))
        .collect();
    assert_eq!(names, [("a_position", 0, 0x8B52), ("a_uv", 1, 0x8B50)]);
    let uniforms: Vec<(&str, u32, u32)> = p
        .uniforms
        .iter()
        .map(|u| (u.name.as_str(), u.gl_type, u.offset))
        .collect();
    // After the translation's own 32 bytes.
    assert_eq!(uniforms, [("u_matrix", 0x8B5C, 32), ("u_tint", 0x8B52, 96)]);
    assert_eq!(p.samplers.len(), 1);
    assert_eq!(
        (
            p.samplers[0].name.as_str(),
            p.samplers[0].gl_type,
            p.samplers[0].bindings.clone()
        ),
        ("u_texture", 0x8B5E, vec![1])
    );
    assert_eq!(p.uniform_size, 112);
    assert!(p.points.is_none());
    assert_eq!(p.outputs.len(), 1);
}

#[test]
fn errors_are_said_as_the_reference_says_them() {
    let fs = "precision mediump float;\nvoid main() { gl_FragColor = vec4(undefinedThing); }\n";
    assert_eq!(
        error(fs, Stage::Fragment, &webgl1()),
        "ERROR: 0:2: 'undefinedThing' : undeclared identifier\n"
    );
    // A block's member of type void, as a struct's (glsl_parse found it
    // made WGSL of no type).
    assert_eq!(
        error(
            "#version 300 es\nuniform Material { void tint; bool lit; };\nvoid main() {}\n",
            Stage::Vertex,
            &webgl2()
        ),
        "ERROR: 0:2: 'void' : illegal use of type 'void'\n"
    );
    // An array of void, at the top level and in a function (glsl_parse
    // found it made WGSL of no type).
    for (source, line) in [
        (
            "void m[2];
void main() {}
",
            2,
        ),
        (
            "void main() {
void m[2];
}
",
            3,
        ),
    ] {
        assert_eq!(
            error(
                &format!(
                    "#version 300 es
{source}"
                ),
                Stage::Vertex,
                &webgl2()
            ),
            format!(
                "ERROR: 0:{line}: 'm' : illegal use of type 'void'
"
            )
        );
    }
    // A block with no member, as a struct with no field (glsl_parse found
    // it made WGSL of an empty struct).
    assert_eq!(
        error(
            "#version 300 es\nuniform Material { };\nvoid main() {}\n",
            Stage::Vertex,
            &webgl2()
        ),
        "ERROR: 0:2: '}' : syntax error\n"
    );
    // An expression of more binary operators than MAX_GLSL_OPERATORS is
    // refused (the Khronos suite's long expressions overflowed the
    // stack); as many in statements of their own are not.
    let sum = |n: usize| vec!["x"; n + 1].join(" + ");
    let long = format!(
        "void main() {{ float x = 1.0; x = {}; gl_Position = vec4(x); }}",
        sum(crate::MAX_GLSL_OPERATORS + 1)
    );
    assert!(error(&long, Stage::Vertex, &webgl1()).contains("Expression too complex"));
    let statements = format!(
        "void main() {{ float x = 1.0; x = {0}; x = {0}; gl_Position = vec4(x); }}",
        sum(crate::MAX_GLSL_OPERATORS / 2 + 10)
    );
    compile(&statements, Stage::Vertex, &webgl1()).unwrap();
    // No default precision: a float of the shader's own is refused, the
    // built-ins' are not.
    compile(
        "void main() { gl_FragColor = vec4(1.0); }",
        Stage::Fragment,
        &webgl1(),
    )
    .unwrap();
    assert_eq!(
        error(
            "void main() { float x = 1.0; gl_FragColor = vec4(x); }",
            Stage::Fragment,
            &webgl1()
        ),
        "ERROR: 0:1: '' : No precision specified for (float)\n"
    );
    assert!(error("void f() {}", Stage::Vertex, &webgl1()).contains("Missing main()"));
    assert!(
        error("void main() { int x = 1.0; }", Stage::Vertex, &webgl1())
            .contains("cannot convert from 'float' to 'int'")
    );
    assert!(
        error(
            "void main() { vec3 v = vec3(1.0) + vec2(1.0); }",
            Stage::Vertex,
            &webgl1()
        )
        .contains("wrong operand types")
    );
    assert!(
        error("void webgl_f() {} void main() {}", Stage::Vertex, &webgl1())
            .contains("reserved built-in name")
    );
    assert!(
        error("void main() { while (true) {} }", Stage::Vertex, &webgl1())
            .contains("This type of loop is not allowed")
    );
    assert!(
        error("void f(); void main() { f(); }", Stage::Vertex, &webgl1())
            .contains("Function declared but not defined")
    );
    assert!(
        error(
            "void f() { f(); } void main() { f(); }",
            Stage::Vertex,
            &webgl1()
        )
        .contains("Function recursion detected")
    );
    assert!(
        error(
            "#extension GL_FOO : require\nvoid main() {}",
            Stage::Vertex,
            &webgl1()
        )
        .contains("extension is not supported")
    );
    assert!(
        error("#version 300 es\nvoid main() {}", Stage::Vertex, &webgl1())
            .contains("unsupported shader version")
    );
    assert!(
        error(
            "void main() { float a[2]; a[2] = 1.0; }",
            Stage::Vertex,
            &webgl1()
        )
        .contains("array index out of range")
    );
    assert!(
        error("void main() { int a = 1 / 0; }", Stage::Vertex, &webgl1())
            .contains("Divide by zero")
    );
    assert!(
        error(
            "void main() { gl_Position = vec4(1.0) $ }",
            Stage::Vertex,
            &webgl1()
        )
        .contains("invalid character")
    );
}

#[test]
fn appendix_a_holds_loops_to_their_form() {
    let ok = "void main() { float s = 0.0; for (int i = 0; i < 4; i++) { s += float(i); } gl_Position = vec4(s); }";
    compile(ok, Stage::Vertex, &webgl1()).unwrap();
    for bad in [
        "void main() { int n = 4; for (int i = 0; i < n; i++) {} }",
        "void main() { for (int i = 0; i < 4; i++) { i = 2; } }",
        "void main() { int i; for (i = 0; i < 4; i++) {} }",
        "void main() { for (int i = 0; i < 4; i *= 2) {} }",
    ] {
        assert!(compile(bad, Stage::Vertex, &webgl1()).is_err(), "{bad}");
    }
}

#[test]
fn the_preprocessor_expands_macros_and_takes_conditionals() {
    let fs = "#define COLOR(r, g) vec4(r, g, 0.0, 1.0)\n\
        #define ONE 1\n\
        #if defined(ONE) && ONE > 0 && GL_ES\n\
        precision highp float;\n\
        #else\n\
        this is not compiled\n\
        #endif\n\
        #ifdef GL_FRAGMENT_PRECISION_HIGH\n\
        void main() { gl_FragColor = COLOR(1.0, float(__LINE__)); }\n\
        #endif\n";
    program(TRIANGLE_VS, fs, &webgl1());
    assert!(
        error(
            "#if UNDEFINED\n#endif\nvoid main() {}",
            Stage::Vertex,
            &webgl1()
        )
        .contains("unexpected token")
    );
    assert!(error("#define GL_X 1\nvoid main() {}", Stage::Vertex, &webgl1()).contains("reserved"));
    assert!(
        error("#error stop here\nvoid main() {}", Stage::Vertex, &webgl1())
            .contains("'#error' : stop here")
    );
    assert!(
        error("#if 1\nvoid main() {}", Stage::Vertex, &webgl1()).contains("unexpected end of file")
    );
}

#[test]
fn the_language_translates_and_validates() {
    let vs = "attribute vec3 a_p;\n\
        uniform float u_values[4];\n\
        uniform bool u_flag;\n\
        uniform mat2 u_m2;\n\
        struct Light { vec3 color; float power; };\n\
        uniform Light u_light;\n\
        uniform Light u_lights[2];\n\
        varying vec3 v_color;\n\
        varying mat2 v_m;\n\
        float counter;\n\
        void bump(inout float x, out float y) { x += 1.0; y = x * 2.0; }\n\
        vec3 shade(Light l) { return l.color * l.power; }\n\
        void main() {\n\
          float a = 0.0;\n\
          for (int i = 0; i < 4; i++) { a += u_values[i]; }\n\
          float b;\n\
          bump(a, b);\n\
          vec4 v = vec4(a, b, 0.0, 1.0);\n\
          v.zx = vec2(3.0, 4.0);\n\
          v.y += v.x++ * --counter;\n\
          bool t = u_flag && (counter++ > 0.0);\n\
          v.w = t ? 1.0 : (counter = 2.0);\n\
          mat3 m = mat3(1.0);\n\
          mat2 n = mat2(m);\n\
          n = n * u_m2 + mat2(v.xy, v.zw) / 2.0;\n\
          v_m = n;\n\
          v_color = shade(u_light) + shade(u_lights[1]) + vec3(mod(a, 2.0), mod(v.xy, 3.0));\n\
          gl_Position = vec4(a_p, 1.0) + v;\n\
          gl_PointSize = 4.0;\n\
        }\n";
    let fs = "precision mediump float;\n\
        varying vec3 v_color;\n\
        varying mat2 v_m;\n\
        uniform samplerCube u_cube;\n\
        uniform sampler2D u_maps[2];\n\
        void main() {\n\
          vec4 c = textureCube(u_cube, v_color);\n\
          for (int i = 0; i < 2; i++) { c += texture2D(u_maps[i], v_m[0]); }\n\
          if (c.a < 0.5) discard;\n\
          gl_FragColor = vec4(v_color, 1.0) * c + vec4(gl_FragCoord.xy, gl_PointCoord) * (gl_FrontFacing ? 1.0 : 0.0);\n\
        }\n";
    let p = program(vs, fs, &webgl1());
    assert!(
        p.points.is_some(),
        "a vertex shader writing gl_PointSize has a points variant"
    );
    let find = |name: &str| {
        p.uniforms
            .iter()
            .find(|u| u.name == name)
            .unwrap_or_else(|| panic!("{name}: {:?}", p.uniforms))
            .clone()
    };
    // An array of floats, each in a vec4.
    let values = find("u_values[0]");
    assert_eq!((values.size, values.array_stride), (4, 16));
    assert_eq!(find("u_flag").gl_type, 0x8B56);
    assert_eq!(find("u_m2").matrix_stride, 16);
    // A struct of a vec3 and a float is 16 bytes, as std140 has it.
    assert_eq!(
        find("u_lights[1].power").offset,
        find("u_lights[0].power").offset + 16
    );
    assert_eq!(
        find("u_light.power").offset,
        find("u_light.color").offset + 12
    );
    let maps = p
        .samplers
        .iter()
        .find(|s| s.name == "u_maps[0]")
        .expect("the array");
    assert_eq!(maps.size, 2);
}

#[test]
fn glsl_es_3_00_translates_its_own() {
    let vs = "#version 300 es\n\
        layout(location = 2) in vec4 a_position;\n\
        in uvec2 a_ids;\n\
        flat out uint v_id;\n\
        out vec2 v_uv[2];\n\
        layout(std140) uniform Scene { mat4 view; vec3 sun; float time; mat2 small; float list[3]; } scene;\n\
        uniform Material { vec4 tint; bool lit; };\n\
        void main() {\n\
          v_id = -a_ids.x ^ ((-a_ids).y << 2u);\n\
          v_uv[0] = a_position.xy;\n\
          v_uv[1] = vec2(float(gl_VertexID), float(gl_InstanceID)) * scene.list[1];\n\
          gl_Position = scene.view * a_position + vec4(scene.sun * scene.time, 0.0) + tint * (lit ? 1.0 : 0.0);\n\
          gl_Position.xy = scene.small * gl_Position.xy;\n\
        }\n";
    let fs = "#version 300 es\n\
        precision highp float;\n\
        precision highp isampler2D;\n\
        precision highp sampler2DArray;\n\
        precision highp sampler2DShadow;\n\
        flat in uint v_id;\n\
        in vec2 v_uv[2];\n\
        uniform isampler2D u_ints;\n\
        uniform sampler2DArray u_layers;\n\
        uniform sampler2DShadow u_shadow;\n\
        layout(location = 0) out vec4 color;\n\
        layout(location = 1) out ivec4 ids;\n\
        int pick(uint id) {\n\
          switch (int(id % 4u)) {\n\
            case 0: return 1;\n\
            case 1:\n\
            case 2: { int k = 2; k += 1; return k; }\n\
            default: break;\n\
          }\n\
          int n = 0;\n\
          switch (int(id)) { case 5: n = 1; case 6: n += 2; break; default: n = 7; }\n\
          return n;\n\
        }\n\
        void main() {\n\
          ivec2 size = textureSize(u_ints, 0);\n\
          ids = texelFetch(u_ints, ivec2(v_uv[0]) % size, 0) + ivec4(pick(v_id));\n\
          color = texture(u_layers, vec3(v_uv[1], 1.0)) * texture(u_shadow, vec3(v_uv[0], 0.5));\n\
          color += textureLod(u_layers, vec3(v_uv[0], 0.0), 1.0) + vec4(uintBitsToFloat(v_id));\n\
          int i = 0;\n\
          do { i++; } while (i < 3);\n\
          while (i > 0) { i -= 1; if (i == 1) continue; color.r += 0.1; }\n\
          gl_FragDepth = color.r;\n\
        }\n";
    let p = program(vs, fs, &webgl2());
    assert_eq!(p.attributes[0].location, 0);
    assert_eq!(
        p.attributes
            .iter()
            .find(|a| a.name == "a_position")
            .map(|a| a.location),
        Some(2)
    );
    assert!(p.writes_depth);
    assert_eq!(
        p.outputs.iter().map(|o| o.location).collect::<Vec<_>>(),
        [0, 1]
    );
    // std140's offsets.
    let find = |name: &str| {
        p.uniforms
            .iter()
            .find(|u| u.name == name)
            .unwrap_or_else(|| panic!("{name}: {:?}", p.uniforms))
            .clone()
    };
    assert_eq!(find("Scene.view").offset, 0);
    assert_eq!(find("Scene.sun").offset, 64);
    assert_eq!(find("Scene.time").offset, 76);
    assert_eq!(find("Scene.small").offset, 80);
    assert_eq!(find("Scene.list[0]").offset, 112);
    assert_eq!(find("Scene.list[0]").array_stride, 16);
    assert_eq!(find("lit").offset, 16);
    let scene = p
        .blocks
        .iter()
        .find(|b| b.name == "Scene")
        .expect("the block");
    assert_eq!(scene.size, 160);
}

#[test]
fn a_link_matches_varyings_and_types() {
    let vs = "varying vec2 v_a; void main() { gl_Position = vec4(0.0); }";
    let fs_missing = "precision mediump float; varying vec2 v_b; void main() { gl_FragColor = vec4(v_b, 0.0, 1.0); }";
    let fs_type =
        "precision mediump float; varying vec3 v_a; void main() { gl_FragColor = vec4(v_a, 1.0); }";
    let v = compile(vs, Stage::Vertex, &webgl1()).unwrap();
    for (fs, why) in [
        (fs_missing, "not found in vertex shader"),
        (fs_type, "differ between vertex and fragment shaders"),
    ] {
        let f = compile(fs, Stage::Fragment, &webgl1()).unwrap();
        let log = link(&v, &f, &LinkOptions::default()).expect_err(fs);
        assert!(log.contains(why), "{log}");
    }
    // Bound attribute locations are kept.
    let v = compile(TRIANGLE_VS, Stage::Vertex, &webgl1()).unwrap();
    let f = compile(TRIANGLE_FS, Stage::Fragment, &webgl1()).unwrap();
    let options = LinkOptions {
        attribute_locations: vec![("a_uv".to_owned(), 5)],
        ..LinkOptions::default()
    };
    let p = link(&v, &f, &options).unwrap();
    let locations: Vec<(String, u32)> = p
        .attributes
        .iter()
        .map(|a| (a.name.clone(), a.location))
        .collect();
    assert_eq!(
        locations,
        [("a_position".to_owned(), 0), ("a_uv".to_owned(), 5)]
    );
}

#[test]
fn a_partner_of_either_stage_compiles_and_links() {
    let es3_fs = "#version 300 es\nprecision highp float;\nflat in uint v_id;\nin vec2 v_uv[2];\nin mat2 v_m;\nout vec4 color;\nvoid main() { color = vec4(float(v_id), v_uv[1], v_m[0][1]); }\n";
    let es3_vs = "#version 300 es\nin vec4 a;\nvoid main() { gl_Position = a; }\n";
    for (source, stage, options) in [
        (TRIANGLE_VS, Stage::Vertex, webgl1()),
        (TRIANGLE_FS, Stage::Fragment, webgl1()),
        (es3_vs, Stage::Vertex, webgl2()),
        (es3_fs, Stage::Fragment, webgl2()),
    ] {
        let shader = compile(source, stage, &options).unwrap();
        let other_stage = if stage == Stage::Vertex {
            Stage::Fragment
        } else {
            Stage::Vertex
        };
        let other = compile(&crate::partner(&shader), other_stage, &options)
            .unwrap_or_else(|log| panic!("{log}"));
        let (v, f) = if stage == Stage::Vertex {
            (&shader, &other)
        } else {
            (&other, &shader)
        };
        link(v, f, &LinkOptions::default()).unwrap_or_else(|log| panic!("{log}"));
    }
}
