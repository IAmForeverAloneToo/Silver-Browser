//! The 2D context through a page's scripts (M30.1): each answer as the
//! reference browser gave it to the same calls (scratchpad m30
//! `p1-canvas.html` and `p3-bindings.html`, Chrome 154), members not yet
//! made (text, images, export) left out; and the bounds a document's
//! canvases keep together.

use js::{Answer, CanvasCall};

use super::canvas::{Canvases, MAX_CANVAS_OBJECTS, MAX_CANVAS_POINTS, MAX_KEPT_COLOURS};
use crate::LoadState;
use crate::testing::*;

/// The lines `script` logs, run on a page after a helper that logs each
/// step as `name=` and its value in JSON, or what it threw.
fn run(script: &str) -> Vec<String> {
    let page = format!(
        "<!DOCTYPE html><canvas id=c width=20 height=10></canvas><script>\n\
         const step = (name, f) => {{ let v; try {{ v = JSON.stringify(f()); }} \
         catch (e) {{ v = 'threw ' + e.name + ': ' + e.message; }} \
         console.log(name + '=' + v); }};\n\
         const err = (e) => 'threw ' + (e && e.name) + ': ' + (e && e.message);\n\
         const px = (ctx, x, y) => Array.from(ctx.getImageData(x, y, 1, 1).data);\n\
         const fresh = (w, h) => {{ const c = document.createElement('canvas'); \
         c.width = w || 10; c.height = h || 10; return c; }};\n\
         const own = (p) => Object.getOwnPropertyNames(p).sort().join(' ');\n\
         {script}</script>"
    );
    let server = serve(move |path| match path {
        "/page.html" => Some(html(&page)),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let url = server.url("/page.html");
    tab.engine.navigate(url.clone());
    tab.wait_until(|engine| {
        incoming(engine).is_none()
            && engine.url() == Some(&url)
            && engine.load_state() == &LoadState::Loaded
    });
    tab.engine
        .take_console()
        .into_iter()
        .map(|entry| entry.text)
        .collect()
}

/// Each `name=value` line of `lines` checked against `want`.
#[track_caller]
fn check(lines: &[String], want: &[(&str, &str)]) {
    for (name, value) in want {
        let prefix = format!("{name}=");
        let line = lines
            .iter()
            .find(|line| line.starts_with(&prefix))
            .unwrap_or_else(|| panic!("no {name} in {lines:#?}"));
        let got = &line[prefix.len()..];
        assert!(close(got, value), "{name}\n  left: {got}\n right: {value}");
    }
}

/// Whether `a` and `b` say the same, their numbers within a few units in
/// the last place of each other. The C libraries' sines and cosines
/// differ in the last place (the reference's numbers came from
/// Windows's; glibc's and macOS's round some the other way), which no
/// test here asks about.
fn close(a: &str, b: &str) -> bool {
    let (a, b) = (tokens(a), tokens(b));
    a.len() == b.len()
        && a.iter().zip(&b).all(|(x, y)| {
            x == y
                || matches!(
                    (x.parse::<f64>(), y.parse::<f64>()),
                    (Ok(x), Ok(y)) if (x - y).abs() <= 4.0 * f64::EPSILON * x.abs().max(y.abs())
                )
        })
}

/// `text` cut into runs of a number's characters and of the others.
fn tokens(text: &str) -> Vec<&str> {
    let numeric = |c: char| c.is_ascii_digit() || c == '.';
    let mut out = Vec::new();
    let mut start = 0;
    let mut kind = None;
    for (at, c) in text.char_indices() {
        let now = numeric(c);
        if kind.is_some_and(|was| was != now) {
            out.push(&text[start..at]);
            start = at;
        }
        kind = Some(now);
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

#[test]
fn numbers_a_last_place_apart_are_close_and_others_are_not() {
    assert!(close(
        "matrix(0.7071067811865476, 0.7071067811865475)",
        "matrix(0.7071067811865476, 0.7071067811865476)"
    ));
    assert!(!close("matrix(0.70710678, 1)", "matrix(0.70710679, 1)"));
    assert!(!close("[1,2]", "[1,2,3]"));
    assert!(!close("a 1", "b 1"));
}

#[test]
fn the_interfaces_have_the_reference_browsers_members() {
    let lines = run(r##"
step("ctxProto", () => own(CanvasRenderingContext2D.prototype));
step("path2dProto", () => own(Path2D.prototype));
step("gradientProto", () => own(CanvasGradient.prototype));
step("patternProto", () => own(CanvasPattern.prototype));
step("imageDataProto", () => own(ImageData.prototype));
step("matrixRO", () => own(DOMMatrixReadOnly.prototype));
step("matrix", () => own(DOMMatrix.prototype));
step("matrixStatic", () => [own(DOMMatrixReadOnly), own(DOMMatrix)]);
step("pointRO", () => own(DOMPointReadOnly.prototype) + " | " + own(DOMPointReadOnly));
step("point", () => own(DOMPoint.prototype));
step("canvasProto", () => own(HTMLCanvasElement.prototype));
step("attrs", () => JSON.stringify(fresh().getContext("2d").getContextAttributes()));
step("attrsOpaque", () => JSON.stringify(fresh().getContext("2d", { alpha: false, willReadFrequently: true }).getContextAttributes()));
step("getContextOther", () => { const c = fresh(); const gl = c.getContext("webgl"); return [gl === null || gl instanceof WebGLRenderingContext, c.getContext("2d") === c.getContext("2d"), c.getContext("bitmaprenderer"), c.getContext("bogus")]; });
step("getContextArgs", () => { try { fresh().getContext(); } catch (e) { return err(e); } });
step("ctxCtor", () => { try { new CanvasRenderingContext2D(); } catch (e) { return err(e); } });
step("ctxCanvas", () => { const c = fresh(); return c.getContext("2d").canvas === c; });
step("toStringTags", () => [String(fresh().getContext("2d")), String(new Path2D()), String(fresh().getContext("2d").createLinearGradient(0,0,1,1)), String(new ImageData(1, 1)), String(new DOMMatrix()), String(new DOMPoint())]);
step("lengths", () => ["fillRect", "arc", "ellipse", "arcTo", "setTransform", "transform", "fill", "stroke", "clip", "isPointInPath", "isPointInStroke", "getImageData", "putImageData", "createImageData", "roundRect", "createConicGradient", "createRadialGradient"].map((n) => n + ":" + CanvasRenderingContext2D.prototype[n].length));
step("matrixLengths", () => ["translate", "scale", "rotate", "multiply", "fromMatrix", "transformPoint"].map((n) => n + ":" + (DOMMatrix.prototype[n] || DOMMatrix[n]).length) + " ctor:" + DOMMatrix.length + " point:" + DOMPoint.length + " ImageData:" + ImageData.length + " Path2D:" + Path2D.length);
"##);
    check(
        &lines,
        &[
            // Chrome's, all 73 and the constructor.
            (
                "ctxProto",
                "\"arc arcTo beginPath bezierCurveTo canvas clearRect clip closePath constructor \
                 createConicGradient createImageData createLinearGradient createPattern \
                 createRadialGradient direction drawFocusIfNeeded drawImage ellipse fill fillRect \
                 fillStyle fillText filter font fontKerning fontStretch fontVariantCaps \
                 getContextAttributes getImageData getLineDash getTransform globalAlpha \
                 globalCompositeOperation imageSmoothingEnabled imageSmoothingQuality \
                 isContextLost isPointInPath isPointInStroke lang letterSpacing lineCap \
                 lineDashOffset lineJoin lineTo lineWidth measureText miterLimit moveTo \
                 putImageData quadraticCurveTo rect reset resetTransform restore rotate roundRect \
                 save scale setLineDash setTransform shadowBlur shadowColor shadowOffsetX \
                 shadowOffsetY stroke strokeRect strokeStyle strokeText textAlign textBaseline \
                 textRendering transform translate wordSpacing\"",
            ),
            (
                "path2dProto",
                "\"addPath arc arcTo bezierCurveTo closePath constructor ellipse lineTo moveTo \
                 quadraticCurveTo rect roundRect\"",
            ),
            ("gradientProto", "\"addColorStop constructor\""),
            ("patternProto", "\"constructor setTransform\""),
            (
                "imageDataProto",
                "\"colorSpace constructor data height pixelFormat width\"",
            ),
            (
                "matrixRO",
                "\"a b c constructor d e f flipX flipY inverse is2D isIdentity m11 m12 m13 m14 \
                 m21 m22 m23 m24 m31 m32 m33 m34 m41 m42 m43 m44 multiply rotate rotateAxisAngle \
                 rotateFromVector scale scale3d scaleNonUniform skewX skewY toFloat32Array \
                 toFloat64Array toJSON toString transformPoint translate\"",
            ),
            (
                "matrix",
                "\"a b c constructor d e f invertSelf m11 m12 m13 m14 m21 m22 m23 m24 m31 m32 \
                 m33 m34 m41 m42 m43 m44 multiplySelf preMultiplySelf rotateAxisAngleSelf \
                 rotateFromVectorSelf rotateSelf scale3dSelf scaleSelf setMatrixValue skewXSelf \
                 skewYSelf translateSelf\"",
            ),
            (
                "matrixStatic",
                "[\"fromFloat32Array fromFloat64Array fromMatrix length name prototype\",\
                 \"fromFloat32Array fromFloat64Array fromMatrix length name prototype\"]",
            ),
            (
                "pointRO",
                "\"constructor matrixTransform toJSON w x y z | fromPoint length name prototype\"",
            ),
            ("point", "\"constructor w x y z\""),
            // Chrome's but captureStream (the design note's section 2);
            // transferControlToOffscreen came with M31.4.
            (
                "canvasProto",
                "\"constructor getContext height toBlob toDataURL transferControlToOffscreen width\"",
            ),
            (
                "attrs",
                "\"{\\\"alpha\\\":true,\\\"colorSpace\\\":\\\"srgb\\\",\\\"colorType\\\":\\\"unorm8\\\",\
                 \\\"desynchronized\\\":false,\\\"toneMapping\\\":{\\\"mode\\\":\\\"standard\\\"},\
                 \\\"willReadFrequently\\\":false}\"",
            ),
            (
                "attrsOpaque",
                "\"{\\\"alpha\\\":false,\\\"colorSpace\\\":\\\"srgb\\\",\\\"colorType\\\":\\\"unorm8\\\",\
                 \\\"desynchronized\\\":false,\\\"toneMapping\\\":{\\\"mode\\\":\\\"standard\\\"},\
                 \\\"willReadFrequently\\\":true}\"",
            ),
            // WebGL is a context (M41), or null without a GPU; the
            // canvas then has no 2D one.
            ("getContextOther", "[true,true,null,null]"),
            (
                "getContextArgs",
                "\"threw TypeError: Failed to execute 'getContext' on 'HTMLCanvasElement': \
                 1 argument required, but only 0 present.\"",
            ),
            (
                "ctxCtor",
                "\"threw TypeError: Failed to construct 'CanvasRenderingContext2D': Illegal \
                 constructor\"",
            ),
            ("ctxCanvas", "true"),
            (
                "toStringTags",
                "[\"[object CanvasRenderingContext2D]\",\"[object Path2D]\",\
                 \"[object CanvasGradient]\",\"[object ImageData]\",\
                 \"matrix(1, 0, 0, 1, 0, 0)\",\"[object DOMPoint]\"]",
            ),
            (
                "lengths",
                "[\"fillRect:4\",\"arc:5\",\"ellipse:7\",\"arcTo:5\",\"setTransform:0\",\
                 \"transform:6\",\"fill:0\",\"stroke:0\",\"clip:0\",\"isPointInPath:2\",\
                 \"isPointInStroke:2\",\"getImageData:4\",\"putImageData:3\",\
                 \"createImageData:1\",\"roundRect:4\",\"createConicGradient:3\",\
                 \"createRadialGradient:6\"]",
            ),
            (
                "matrixLengths",
                "\"translate:0,scale:0,rotate:0,multiply:0,fromMatrix:0,transformPoint:0 ctor:0 \
                 point:0 ImageData:2 Path2D:0\"",
            ),
        ],
    );
}

#[test]
fn drawing_through_the_bindings_reads_back_as_the_reference_browser_drew_it() {
    let lines = run(r##"
step("fillRect", () => { const x = fresh().getContext("2d"); x.fillStyle = "rgb(10, 20, 30)"; x.fillRect(2, 2, 4, 4);
  return [px(x, 1, 1), px(x, 2, 2), px(x, 5, 5), px(x, 6, 6)]; });
step("alpha", () => { const x = fresh().getContext("2d"); x.fillStyle = "rgba(255, 128, 0, 0.5)"; x.fillRect(0, 0, 10, 10);
  x.globalAlpha = 0.5; x.fillStyle = "#0000ff"; x.fillRect(5, 0, 5, 10); return px(x, 1, 1); });
step("premultiplied", () => { const x = fresh().getContext("2d"); const d = x.createImageData(1, 1);
  d.data.set([255, 128, 7, 1]); x.putImageData(d, 0, 0); d.data.set([200, 100, 50, 128]); x.putImageData(d, 1, 0);
  d.data.set([255, 255, 255, 0]); x.putImageData(d, 2, 0); return [px(x, 0, 0), px(x, 1, 0), px(x, 2, 0)]; });
step("composite", () => { const r = []; for (const op of ["source-over", "copy", "destination-out", "xor", "source-in"]) {
  const x = fresh().getContext("2d"); x.fillStyle = "rgba(255, 0, 0, 0.75)"; x.fillRect(0, 0, 10, 10);
  x.globalCompositeOperation = op; x.fillStyle = "rgba(0, 0, 255, 0.5)"; x.fillRect(0, 0, 5, 10); r.push(op + ":" + px(x, 1, 1) + "|" + px(x, 7, 1)); } return r; });
step("stroke", () => { const x = fresh(20, 20).getContext("2d"); x.lineWidth = 2; x.strokeStyle = "#000"; x.beginPath();
  x.moveTo(2, 10); x.lineTo(18, 10); x.stroke(); return [px(x, 10, 8), px(x, 10, 9), px(x, 10, 10), px(x, 10, 11), px(x, 1, 10), px(x, 18, 10)]; });
step("gradient", () => { const x = fresh(10, 1).getContext("2d"); const g = x.createLinearGradient(0, 0, 10, 0); g.addColorStop(0, "#000"); g.addColorStop(1, "#fff");
  x.fillStyle = g; const s = String(x.fillStyle); x.fillRect(0, 0, 10, 1); return [s, px(x, 5, 0), px(x, 9, 0)]; });
step("rect", () => { const x = fresh(20, 20).getContext("2d"); x.beginPath(); x.rect(2, 2, 6, 6); x.roundRect(10, 10, 8, 8, 3); x.fill();
  return [px(x, 4, 4), px(x, 14, 14)]; });
step("clip", () => { const x = fresh(20, 20).getContext("2d"); x.beginPath(); x.rect(5, 5, 10, 10); x.clip(); x.fillRect(0, 0, 20, 20);
  return [px(x, 2, 2), px(x, 7, 7)]; });
step("pointInPath", () => { const x = fresh(20, 20).getContext("2d"); x.beginPath(); x.rect(5, 5, 10, 10);
  return [x.isPointInPath(6, 6), x.isPointInPath(1, 1), x.isPointInPath(15, 15), x.isPointInStroke(5, 10), typeof Path2D,
    x.isPointInPath(new Path2D("M0 0 L10 0 L10 10 Z"), 8, 2)]; });
step("resetOnResize", () => { const c = fresh(); const x = c.getContext("2d"); x.fillStyle = "red"; x.fillRect(0, 0, 10, 10); x.translate(3, 3); c.width = 10;
  return [px(x, 1, 1), x.fillStyle, x.getTransform().e]; });
step("widthClears", () => { const c = fresh(); const x = c.getContext("2d"); x.fillRect(0, 0, 10, 10); c.setAttribute("height", "10"); const a = px(x, 1, 1);
  x.fillRect(0, 0, 10, 10); c.setAttribute("data-x", "1"); return [a, px(x, 1, 1)]; });
step("shadow", () => { const x = fresh(20, 20).getContext("2d"); x.shadowColor = "rgba(0,0,0,1)"; x.shadowOffsetX = 5; x.fillStyle = "#f00"; x.fillRect(2, 2, 5, 5);
  return [px(x, 3, 3), px(x, 9, 3), px(x, 13, 3)]; });
step("opaquePixel", () => { const x = fresh().getContext("2d", { alpha: false }); x.clearRect(0, 0, 5, 5);
  x.fillStyle = "rgba(255,255,255,0.5)"; x.fillRect(0, 0, 10, 10); return px(x, 1, 1); });
step("fillArgs", () => { const x = fresh(20, 20).getContext("2d"); const r = []; x.rect(0, 0, 10, 10); x.rect(2, 2, 6, 6);
  x.fill("evenodd"); r.push(px(x, 5, 5)[3], px(x, 1, 1)[3]); r.push(x.isPointInPath(5, 5, "evenodd"), x.isPointInPath(new Path2D("M0 0h4v4z"), 1, 3)); return r; });
step("path2d", () => { const x = fresh(20, 20).getContext("2d"); const r = []; const p = new Path2D("M2 2 L8 2 L8 8 Z m10 0 h4 v4 h-4 z");
  x.fill(p); r.push(px(x, 6, 4)[3], px(x, 13, 3)[3], px(x, 3, 6)[3]); const q = new Path2D(p); q.rect(0, 15, 3, 3); x.fill(q); r.push(px(x, 1, 16)[3]);
  const bad = new Path2D("M 2 2 L 10 2 L 10 x 5 L 2 10 Z"); x.clearRect(0, 0, 20, 20); x.fill(bad); r.push(px(x, 5, 3)[3]);
  const arc = new Path2D("M 2 10 A 8 8 0 0 1 18 10 Z"); x.clearRect(0, 0, 20, 20); x.fill(arc); r.push(px(x, 10, 5)[3], px(x, 10, 15)[3]);
  const add = new Path2D(); add.addPath(new Path2D("M0 0h2v2h-2z"), { e: 5, f: 5 }); x.clearRect(0, 0, 20, 20); x.fill(add); r.push(px(x, 6, 6)[3], px(x, 1, 1)[3]);
  try { add.addPath({}); } catch (e) { r.push(err(e)); } try { new Path2D(5); r.push("number ok"); } catch (e) { r.push(err(e)); }
  const s = new Path2D("M1,1L5,1 5,5zM 10 10 Q 15 10 15 15 T 18 18 C 1 2 3 4 5 6 S 7 8 9 9 A 2 2 0 1 0 12 12 1e1 5"); r.push("parsed");
  const flags = new Path2D("M0 0a5 5 0 1012 0"); x.clearRect(0, 0, 20, 20); x.fill(flags); r.push(px(x, 6, 3)[3]); return r; });
step("states", () => { const x = fresh().getContext("2d"); const r = []; x.lineWidth = 5; x.lineCap = "round"; x.globalAlpha = 0.5;
  x.reset(); r.push(x.lineWidth, x.lineCap, x.globalAlpha); for (let i = 0; i < 2000; i++) x.save(); x.lineWidth = 3; for (let i = 0; i < 2000; i++) x.restore(); r.push(x.lineWidth); return r; });
step("saveRestore", () => { const x = fresh().getContext("2d"); x.fillStyle = "red"; x.save(); x.fillStyle = "blue"; x.translate(5, 5); x.restore();
  x.restore(); return [x.fillStyle, x.getTransform().e]; });
step("dirty", () => { const x = fresh(4, 4).getContext("2d"); const d = new ImageData(2, 2); d.data.fill(255);
  x.putImageData(d, 1, 1, 1, 1, 1, 1); return [px(x, 1, 1)[3], px(x, 2, 2)[3]]; });
step("booleans", () => { const x = fresh().getContext("2d"); x.imageSmoothingEnabled = false; const a = x.imageSmoothingEnabled;
  x.imageSmoothingEnabled = true; const c = fresh(20, 20).getContext("2d"); c.arc(10, 10, 8, 0, Math.PI / 2, true); c.fill();
  return [a, x.imageSmoothingEnabled, px(c, 6, 6)[3], px(c, 14, 14)[3]]; });
step("longs", () => { const x = fresh().getContext("2d"); const r = [x.getImageData(0.9, 0.9, 1.9, -1.9).height];
  try { x.getImageData(NaN, 0, 1, 1); } catch (e) { r.push(e.name); } try { x.getImageData(0, 0, 1, 2 ** 31); } catch (e) { r.push(e.name); } return r; });
step("dash", () => { const x = fresh().getContext("2d"); x.setLineDash([2, 1]); const a = x.getLineDash(); x.setLineDash([1, 2, 3]); const b = x.getLineDash();
  x.setLineDash([1, -1]); const c = x.getLineDash(); x.setLineDash([1, NaN]); return [a, b, c, x.getLineDash()].map((v) => JSON.stringify(v)); });
"##);
    check(
        &lines,
        &[
            (
                "fillRect",
                "[[0,0,0,0],[10,20,30,255],[10,20,30,255],[0,0,0,0]]",
            ),
            // Over it, blue at half: [85, 43, 170, 192], a level of green
            // from Chrome's 42 (the crate's test allows it).
            ("alpha", "[255,128,0,128]"),
            (
                "premultiplied",
                "[[255,255,0,1],[199,100,50,128],[0,0,0,0]]",
            ),
            (
                "composite",
                "[\"source-over:109,0,146,223|255,0,0,191\",\"copy:0,0,255,128|0,0,0,0\",\
                 \"destination-out:255,0,0,95|255,0,0,191\",\"xor:191,0,64,127|255,0,0,191\",\
                 \"source-in:0,0,255,96|0,0,0,0\"]",
            ),
            (
                "stroke",
                "[[0,0,0,0],[0,0,0,255],[0,0,0,255],[0,0,0,0],[0,0,0,0],[0,0,0,0]]",
            ),
            // Pixel 0 is 13 here, 12 in Chrome (the crate's test allows a
            // level).
            (
                "gradient",
                "[\"[object CanvasGradient]\",[140,140,140,255],[242,242,242,255]]",
            ),
            ("rect", "[[0,0,0,255],[0,0,0,255]]"),
            ("clip", "[[0,0,0,0],[0,0,0,255]]"),
            ("pointInPath", "[true,false,true,true,\"function\",true]"),
            ("resetOnResize", "[[0,0,0,0],\"#000000\",0]"),
            ("widthClears", "[[0,0,0,0],[0,0,0,255]]"),
            ("shadow", "[[255,0,0,255],[0,0,0,255],[0,0,0,0]]"),
            ("opaquePixel", "[128,128,128,255]"),
            ("fillArgs", "[0,255,false,false]"),
            (
                "path2d",
                "[255,255,0,255,0,255,0,255,0,\"threw TypeError: Failed to execute 'addPath' \
                 on 'Path2D': parameter 1 is not of type 'Path2D'.\",\"number ok\",\"parsed\",255]",
            ),
            ("states", "[1,\"butt\",1,1]"),
            ("dirty", "[0,255]"),
            ("booleans", "[false,true,255,0]"),
            ("longs", "[1,\"TypeError\",\"TypeError\"]"),
            ("saveRestore", "[\"#ff0000\",0]"),
            (
                "dash",
                "[\"[2,1]\",\"[1,2,3,1,2,3]\",\"[1,2,3,1,2,3]\",\"[1,2,3,1,2,3]\"]",
            ),
        ],
    );
}

#[test]
fn refusals_are_in_the_reference_browsers_words() {
    let lines = run(r##"
step("styleErrors", () => { const x = fresh().getContext("2d"); const r = []; const g = x.createLinearGradient(0, 0, 1, 1);
  try { g.addColorStop(NaN, "red"); } catch (e) { r.push(err(e)); } try { g.addColorStop(0.5, "bogus"); } catch (e) { r.push(err(e)); }
  try { g.addColorStop(0.5); } catch (e) { r.push(err(e)); } try { g.addColorStop(2, "#000"); } catch (e) { r.push(err(e)); }
  try { x.createRadialGradient(0, 0, 1, 0, 0, -1); } catch (e) { r.push(err(e)); } try { x.createRadialGradient(0, 0, Infinity, 0, 0, 1); } catch (e) { r.push(err(e)); }
  try { x.createConicGradient(NaN, 0, 0); } catch (e) { r.push(err(e)); } r.push(String(x.createConicGradient(0, 0, 0)));
  x.fillStyle = g; r.push(x.fillStyle === g); x.fillStyle = {}; r.push(x.fillStyle === g); x.fillStyle = null; r.push(x.fillStyle === g);
  x.save(); x.fillStyle = "red"; x.restore(); r.push(x.fillStyle === g); x.strokeStyle = "red"; x.strokeStyle = "rgba(0,0,0,0.1234)"; r.push(x.strokeStyle);
  x.shadowColor = "red"; x.shadowColor = "nope"; r.push(x.shadowColor); return r; });
step("numberArgs", () => { const x = fresh().getContext("2d"); const r = [];
  try { x.fillRect(1, 2, 3); } catch (e) { r.push(err(e)); } try { x.arc(0, 0, 1, 0); } catch (e) { r.push(err(e)); }
  x.fillRect(Infinity, 0, 1, 1); x.fillRect("1", "1", "2", "2"); r.push(px(x, 1, 1)); try { x.fillRect(Symbol(), 0, 1, 1); } catch (e) { r.push(err(e)); }
  try { x.fillRect(1n, 0, 1, 1); } catch (e) { r.push(err(e)); }
  try { x.lineWidth = Symbol(); } catch (e) { r.push(err(e)); } x.globalCompositeOperation = "Copy"; r.push(x.globalCompositeOperation);
  x.imageSmoothingQuality = "high"; x.imageSmoothingQuality = "best"; r.push(x.imageSmoothingQuality); x.imageSmoothingEnabled = 0; r.push(x.imageSmoothingEnabled);
  r.push(x.isContextLost()); x.filter = "blur(2px)"; r.push(x.filter); x.filter = "bogus"; r.push(x.filter); return r; });
step("fillArgs", () => { const x = fresh(20, 20).getContext("2d"); const r = [];
  try { x.fill("bogus"); } catch (e) { r.push(err(e)); } try { x.fill({}); } catch (e) { r.push(err(e)); } try { x.fill(new Path2D(), "bogus"); } catch (e) { r.push(err(e)); }
  try { x.isPointInPath(1); } catch (e) { r.push(err(e)); } try { x.clip(1); } catch (e) { r.push(err(e)); } try { x.stroke(1); } catch (e) { r.push(err(e)); } return r; });
step("lineDash", () => { const x = fresh().getContext("2d"); const r = []; x.setLineDash([1, 2]); const d = x.getLineDash(); d.push(9); r.push(JSON.stringify(x.getLineDash()));
  try { x.setLineDash(5); } catch (e) { r.push(err(e)); } try { x.setLineDash(); } catch (e) { r.push(err(e)); } x.setLineDash([Infinity]); r.push(JSON.stringify(x.getLineDash()));
  x.setLineDash(["3", 4]); r.push(JSON.stringify(x.getLineDash())); x.lineDashOffset = NaN; r.push(x.lineDashOffset); return r; });
step("roundRectArgs", () => { const x = fresh().getContext("2d"); const r = []; try { x.roundRect(0, 0, 5, 5, []); } catch (e) { r.push(err(e)); }
  try { x.roundRect(0, 0, 5, 5, [1, 2, 3, 4, 5]); } catch (e) { r.push(err(e)); } try { x.roundRect(0, 0, 5, 5, -1); } catch (e) { r.push(err(e)); }
  try { x.roundRect(0, 0, 5, 5, [{ x: 1, y: -1 }]); } catch (e) { r.push(err(e)); } x.roundRect(0, 0, 5, 5); x.roundRect(0, 0, 5, 5, new DOMPoint(1, 2)); r.push("ok");
  try { x.roundRect(0, 0, 5, 5, "3"); r.push("string ok"); } catch (e) { r.push(err(e)); } return r; });
step("arcErrors", () => { const x = fresh().getContext("2d"); const r = []; try { x.arc(0, 0, -1, 0, 1); } catch (e) { r.push(err(e)); }
  try { x.arcTo(0, 0, 1, 1, -1); } catch (e) { r.push(err(e)); } x.arc(NaN, 0, 1, 0, 1); r.push("nan ok"); return r; });
step("imageDataArgs", () => { const x = fresh().getContext("2d"); const r = []; const d = new ImageData(2, 1); r.push(Object.prototype.toString.call(d.data), d.data.length, d.colorSpace);
  r.push(Object.getOwnPropertyDescriptor(ImageData.prototype, "data") ? "accessor" : "own"); try { new ImageData(0, 1); } catch (e) { r.push(err(e)); }
  try { new ImageData(new Uint8ClampedArray(8), 3); } catch (e) { r.push(err(e)); } try { new ImageData(new Uint8ClampedArray(8), 2, 2); } catch (e) { r.push(err(e)); }
  try { new ImageData(new Uint8Array(8), 2); } catch (e) { r.push(err(e)); } try { new ImageData(1); } catch (e) { r.push(err(e)); }
  try { new ImageData(1, 1, { colorSpace: "display-p3" }).colorSpace; r.push("p3 ok"); } catch (e) { r.push(err(e)); }
  r.push(x.createImageData(-2, 3).width);
  r.push(x.createImageData(d).width); try { x.createImageData(); } catch (e) { r.push(err(e)); } try { x.putImageData({}, 0, 0); } catch (e) { r.push(err(e)); }
  try { x.getImageData(0, 0, 1e5, 1e5); } catch (e) { r.push(err(e)); } try { new ImageData(1e5, 1e5); } catch (e) { r.push(err(e)); }
  try { x.getImageData(0, 0, 0, 1); } catch (e) { r.push(err(e)); } try { new ImageData(new Uint8ClampedArray(7), 2); } catch (e) { r.push(err(e)); }
  const g = x.getImageData(0, 0, 1, 1); r.push(g.data.buffer.byteLength, Object.isFrozen(g.data)); return r; });
step("transformArgs", () => { const x = fresh().getContext("2d"); const r = []; x.setTransform(1, 0, 0, 1, 5, 6); r.push(String(x.getTransform()));
  x.setTransform({ a: 2, d: 2 }); r.push(String(x.getTransform())); x.setTransform(); r.push(String(x.getTransform()));
  try { x.setTransform({ a: 1, m11: 2 }); } catch (e) { r.push(err(e)); } try { x.setTransform(1, 2); } catch (e) { r.push(err(e)); }
  x.setTransform(new DOMMatrix([1, 0, 0, 1, 3, 4])); r.push(String(x.getTransform())); x.setTransform(NaN, 0, 0, 1, 0, 0); r.push(String(x.getTransform()));
  try { x.setTransform(new DOMMatrix([1,0,0,0, 0,1,0,0, 0,0,2,0, 0,0,0,1])); r.push(String(x.getTransform())); } catch (e) { r.push(err(e)); }
  x.resetTransform(); x.transform(1, 0, 0, 1, 2, 2); x.transform(2, 0, 0, 2, 0, 0); r.push(String(x.getTransform())); x.getTransform().a = 9; r.push(x.getTransform().a);
  try { x.transform(1, 2); } catch (e) { r.push(err(e)); } try { x.scale(1); } catch (e) { r.push(err(e)); } return r; });
"##);
    check(
        &lines,
        &[
            (
                "styleErrors",
                "[\"threw TypeError: Failed to execute 'addColorStop' on 'CanvasGradient': The \
                 provided double value is non-finite.\",\"threw SyntaxError: Failed to execute \
                 'addColorStop' on 'CanvasGradient': The value provided ('bogus') could not be \
                 parsed as a color.\",\"threw TypeError: Failed to execute 'addColorStop' on \
                 'CanvasGradient': 2 arguments required, but only 1 present.\",\"threw \
                 IndexSizeError: Failed to execute 'addColorStop' on 'CanvasGradient': The \
                 provided value (2) is outside the range (0.0, 1.0).\",\"threw IndexSizeError: \
                 Failed to execute 'createRadialGradient' on 'CanvasRenderingContext2D': The r1 \
                 provided is less than 0.\",\"threw TypeError: Failed to execute \
                 'createRadialGradient' on 'CanvasRenderingContext2D': The provided double value \
                 is non-finite.\",\"threw TypeError: Failed to execute 'createConicGradient' on \
                 'CanvasRenderingContext2D': The provided double value is non-finite.\",\
                 \"[object CanvasGradient]\",true,true,true,true,\"rgba(0, 0, 0, 0.12)\",\
                 \"#ff0000\"]",
            ),
            (
                "numberArgs",
                "[\"threw TypeError: Failed to execute 'fillRect' on 'CanvasRenderingContext2D': \
                 4 arguments required, but only 3 present.\",\"threw TypeError: Failed to execute \
                 'arc' on 'CanvasRenderingContext2D': 5 arguments required, but only 4 present.\",\
                 [0,0,0,255],\"threw TypeError: Failed to execute 'fillRect' on \
                 'CanvasRenderingContext2D': Cannot convert a Symbol value to a number\",\"threw \
                 TypeError: Failed to execute 'fillRect' on 'CanvasRenderingContext2D': Cannot \
                 convert a BigInt value to a number\",\"threw TypeError: Failed to set the \
                 'lineWidth' property on 'CanvasRenderingContext2D': Cannot convert a Symbol \
                 value to a number\",\"source-over\",\"high\",false,false,\"blur(2px)\",\
                 \"blur(2px)\"]",
            ),
            (
                "fillArgs",
                "[\"threw TypeError: Failed to execute 'fill' on 'CanvasRenderingContext2D': The \
                 provided value 'bogus' is not a valid enum value of type CanvasFillRule.\",\
                 \"threw TypeError: Failed to execute 'fill' on 'CanvasRenderingContext2D': The \
                 provided value '[object Object]' is not a valid enum value of type \
                 CanvasFillRule.\",\"threw TypeError: Failed to execute 'fill' on \
                 'CanvasRenderingContext2D': The provided value 'bogus' is not a valid enum value \
                 of type CanvasFillRule.\",\"threw TypeError: Failed to execute 'isPointInPath' \
                 on 'CanvasRenderingContext2D': 2 arguments required, but only 1 present.\",\
                 \"threw TypeError: Failed to execute 'clip' on 'CanvasRenderingContext2D': The \
                 provided value '1' is not a valid enum value of type CanvasFillRule.\",\"threw \
                 TypeError: Failed to execute 'stroke' on 'CanvasRenderingContext2D': parameter 1 \
                 is not of type 'Path2D'.\"]",
            ),
            (
                "lineDash",
                "[\"[1,2]\",\"threw TypeError: Failed to execute 'setLineDash' on \
                 'CanvasRenderingContext2D': The provided value cannot be converted to a \
                 sequence.\",\"threw TypeError: Failed to execute 'setLineDash' on \
                 'CanvasRenderingContext2D': 1 argument required, but only 0 present.\",\
                 \"[1,2]\",\"[3,4]\",0]",
            ),
            (
                "roundRectArgs",
                "[\"threw RangeError: Failed to execute 'roundRect' on \
                 'CanvasRenderingContext2D': 0 radii provided. Between one and four radii are \
                 necessary.\",\"threw RangeError: Failed to execute 'roundRect' on \
                 'CanvasRenderingContext2D': 5 radii provided. Between one and four radii are \
                 necessary.\",\"threw RangeError: Failed to execute 'roundRect' on \
                 'CanvasRenderingContext2D': Radius value -1 is negative.\",\"threw RangeError: \
                 Failed to execute 'roundRect' on 'CanvasRenderingContext2D': Y-radius value -1 \
                 is negative.\",\"ok\",\"string ok\"]",
            ),
            (
                "arcErrors",
                "[\"threw IndexSizeError: Failed to execute 'arc' on 'CanvasRenderingContext2D': \
                 The radius provided (-1) is negative.\",\"threw IndexSizeError: Failed to \
                 execute 'arcTo' on 'CanvasRenderingContext2D': The radius provided (-1) is \
                 negative.\",\"nan ok\"]",
            ),
            (
                "imageDataArgs",
                "[\"[object Uint8ClampedArray]\",8,\"srgb\",\"accessor\",\"threw IndexSizeError: \
                 Failed to construct 'ImageData': The source width is zero or not a number.\",\
                 \"threw IndexSizeError: Failed to construct 'ImageData': The input data length \
                 is not a multiple of (4 * width).\",\"threw IndexSizeError: Failed to construct \
                 'ImageData': The input data length is not equal to (4 * width * height).\",\
                 \"threw IndexSizeError: Failed to construct 'ImageData': The source width is \
                 zero or not a number.\",\"threw TypeError: Failed to construct 'ImageData': 2 \
                 arguments required, but only 1 present.\",\"p3 ok\",2,2,\"threw TypeError: \
                 Failed to execute 'createImageData' on 'CanvasRenderingContext2D': 1 argument \
                 required, but only 0 present.\",\"threw TypeError: Failed to execute \
                 'putImageData' on 'CanvasRenderingContext2D': parameter 1 is not of type \
                 'ImageData'.\",\"threw RangeError: Failed to execute 'getImageData' on \
                 'CanvasRenderingContext2D': Out of memory at ImageData creation\",\"threw \
                 IndexSizeError: Failed to construct 'ImageData': The requested image size \
                 exceeds the supported range.\",\"threw IndexSizeError: Failed to execute \
                 'getImageData' on 'CanvasRenderingContext2D': The source width is 0.\",\"threw \
                 InvalidStateError: Failed to construct 'ImageData': The input data length is not \
                 a multiple of 4.\",4,false]",
            ),
            (
                "transformArgs",
                "[\"matrix(1, 0, 0, 1, 5, 6)\",\"matrix(2, 0, 0, 2, 0, 0)\",\
                 \"matrix(1, 0, 0, 1, 0, 0)\",\"threw TypeError: Failed to execute \
                 'setTransform' on 'CanvasRenderingContext2D': Property mismatch on matrix \
                 initialization.\",\"threw TypeError: Failed to execute 'setTransform' on \
                 'CanvasRenderingContext2D': The provided value is not of type \
                 'DOMMatrixInit'.\",\"matrix(1, 0, 0, 1, 3, 4)\",\"matrix(1, 0, 0, 1, 3, 4)\",\
                 \"matrix(1, 0, 0, 1, 0, 0)\",\"matrix(2, 0, 0, 2, 2, 2)\",2,\"threw TypeError: \
                 Failed to execute 'transform' on 'CanvasRenderingContext2D': 6 arguments \
                 required, but only 2 present.\",\"threw TypeError: Failed to execute 'scale' on \
                 'CanvasRenderingContext2D': 2 arguments required, but only 1 present.\"]",
            ),
        ],
    );
}

#[test]
fn dom_matrix_and_dom_point_compute_as_the_reference_browser_does() {
    let lines = run(r##"
step("matrixMisc", () => { const r = []; const m = new DOMMatrix(); r.push(String(m), m.is2D, m.isIdentity); r.push(String(new DOMMatrix([1,2,3,4,5,6]).inverse()));
  r.push(String(new DOMMatrix("translate(10px, 5px) scale(2)"))); try { new DOMMatrix("bogus"); } catch (e) { r.push(err(e)); }
  try { new DOMMatrix([1, 2]); } catch (e) { r.push(err(e)); } try { new DOMMatrix("translate(1em)"); } catch (e) { r.push(err(e)); }
  r.push(String(new DOMMatrix().rotate(90))); r.push(String(new DOMMatrix().rotate(30))); r.push(String(new DOMMatrix().rotate(10, 20, 30)));
  r.push(String(new DOMMatrix().scale(2, 3, 1, 5, 5))); r.push(String(new DOMMatrix().skewX(45))); r.push(String(new DOMMatrix().translate(1, 2, 3)));
  r.push(String(new DOMMatrix([0, 0, 0, 0, 0, 0]).inverse().a)); try { String(new DOMMatrix([NaN, 0, 0, 1, 0, 0])); } catch (e) { r.push(err(e)); }
  r.push(JSON.stringify(new DOMMatrix([1, 2, 3, 4, 5, 6]).toJSON())); r.push(Array.from(new DOMMatrix([1, 2, 3, 4, 5, 6]).toFloat32Array()).join());
  r.push(JSON.stringify(new DOMMatrix().transformPoint({ x: 1, y: 2 }))); r.push(String(DOMMatrix.fromMatrix({ m11: 2 })));
  try { DOMMatrix.fromMatrix({ is2D: true, m33: 2 }); } catch (e) { r.push(err(e)); } r.push(String(new DOMMatrix().rotateFromVector(1, 1)));
  r.push(String(new DOMMatrix([-0, 0, 0, 1, 0, 0]))); r.push(String(new DOMMatrix().flipX())); const k = new DOMMatrix(); k.m33 = 2; r.push(k.is2D); k.m33 = 1; r.push(k.is2D);
  r.push(String(new DOMMatrix().setMatrixValue("matrix(1, 2, 3, 4, 5, 6)"))); r.push(String(new DOMMatrixReadOnly([1, 0, 0, 1, 2, 3]).translate(1, 1)));
  r.push(Object.prototype.toString.call(new DOMMatrixReadOnly().translate(1, 1))); r.push(String(new DOMMatrix().rotateAxisAngle(0, 0, 1, 90)));
  r.push(String(new DOMMatrix(new Float64Array([1, 0, 0, 1, 7, 8])))); r.push(String(new DOMMatrix().scale3d(2)));
  return r; });
step("point", () => { const r = []; const p = new DOMPoint(1, 2); r.push(JSON.stringify(p), String(p), p.w); r.push(JSON.stringify(DOMPoint.fromPoint({ x: 5 })));
  r.push(JSON.stringify(new DOMPoint(1, 1).matrixTransform(new DOMMatrix([2, 0, 0, 2, 3, 3])))); return r; });
"##);
    check(
        &lines,
        &[
            (
                "matrixMisc",
                "[\"matrix(1, 0, 0, 1, 0, 0)\",true,true,\"matrix(-2, 1, 1.5, -0.5, 1, -2)\",\
                 \"matrix(2, 0, 0, 2, 10, 5)\",\"threw SyntaxError: Failed to construct \
                 'DOMMatrix': Failed to parse 'bogus'.\",\"threw TypeError: Failed to construct \
                 'DOMMatrix': The sequence must contain 6 elements for a 2D matrix or 16 elements \
                 for a 3D matrix.\",\"threw SyntaxError: Failed to construct 'DOMMatrix': Values \
                 must be resolvable at parse time\",\"matrix(0, 1, -1, 0, 0, 0)\",\
                 \"matrix(0.8660254037844387, 0.49999999999999994, -0.49999999999999994, \
                 0.8660254037844387, 0, 0)\",\"matrix3d(0.8137976813493738, 0.46984631039295416, \
                 -0.3420201433256687, 0, -0.44096961052988237, 0.8825641192593856, \
                 0.16317591116653482, 0, 0.37852230636979245, 0.01802831123629725, \
                 0.9254165783983234, 0, 0, 0, 0, 1)\",\"matrix(2, 0, 0, 3, -5, -10)\",\
                 \"matrix(1, 0, 0.9999999999999999, 1, 0, 0)\",\"matrix3d(1, 0, 0, 0, 0, 1, 0, \
                 0, 0, 0, 1, 0, 1, 2, 3, 1)\",\"NaN\",\"threw InvalidStateError: Failed to \
                 execute 'toString' on 'DOMMatrixReadOnly': DOMMatrix cannot be serialized with \
                 NaN or Infinity values.\",\"{\\\"a\\\":1,\\\"b\\\":2,\\\"c\\\":3,\\\"d\\\":4,\
                 \\\"e\\\":5,\\\"f\\\":6,\\\"m11\\\":1,\\\"m12\\\":2,\\\"m13\\\":0,\\\"m14\\\":0,\
                 \\\"m21\\\":3,\\\"m22\\\":4,\\\"m23\\\":0,\\\"m24\\\":0,\\\"m31\\\":0,\
                 \\\"m32\\\":0,\\\"m33\\\":1,\\\"m34\\\":0,\\\"m41\\\":5,\\\"m42\\\":6,\
                 \\\"m43\\\":0,\\\"m44\\\":1,\\\"is2D\\\":true,\\\"isIdentity\\\":false}\",\
                 \"1,2,0,0,3,4,0,0,0,0,1,0,5,6,0,1\",\"{\\\"x\\\":1,\\\"y\\\":2,\\\"z\\\":0,\
                 \\\"w\\\":1}\",\"matrix(2, 0, 0, 1, 0, 0)\",\"threw TypeError: Failed to execute \
                 'fromMatrix' on 'DOMMatrix': The is2D member is set to true but the input matrix \
                 is a 3d matrix.\",\"matrix(0.7071067811865476, 0.7071067811865476, \
                 -0.7071067811865476, 0.7071067811865476, 0, 0)\",\"matrix(0, 0, 0, 1, 0, 0)\",\
                 \"matrix(-1, 0, 0, 1, 0, 0)\",false,false,\"matrix(1, 2, 3, 4, 5, 6)\",\
                 \"matrix(1, 0, 0, 1, 3, 4)\",\"[object DOMMatrix]\",\"matrix(0, 1, -1, 0, 0, \
                 0)\",\"matrix(1, 0, 0, 1, 7, 8)\",\"matrix3d(2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, \
                 0, 0, 0, 1)\"]",
            ),
            (
                "point",
                "[\"{\\\"x\\\":1,\\\"y\\\":2,\\\"z\\\":0,\\\"w\\\":1}\",\"[object DOMPoint]\",1,\
                 \"{\\\"x\\\":5,\\\"y\\\":0,\\\"z\\\":0,\\\"w\\\":1}\",\"{\\\"x\\\":5,\
                 \\\"y\\\":5,\\\"z\\\":0,\\\"w\\\":1}\"]",
            ),
        ],
    );
}

fn call(op: &str, numbers: &[f64], strings: &[&str]) -> CanvasCall {
    CanvasCall {
        op: op.to_owned(),
        numbers: numbers.to_vec(),
        strings: strings.iter().map(|s| (*s).to_owned()).collect(),
        bytes: None,
    }
}

#[test]
fn a_documents_canvas_objects_are_bounded_together() {
    let mut canvases = Canvases::default();
    // A million points a path, its own bound; four of them fill the
    // document's, and a fifth is made empty.
    let d = format!("M0 0{}", "L1 1".repeat(1 << 20));
    let mut lens = Vec::new();
    for _ in 0..5 {
        let Ok(Answer::Number(id)) = canvases.call(call("path", &[], &[&d]), None) else {
            panic!("no path");
        };
        let more = canvases.call(call("lineTo", &[2.0, id, 5.0, 5.0], &[]), None);
        assert!(more.is_ok());
        lens.push(id);
    }
    assert_eq!(MAX_CANVAS_POINTS, 4 << 20);
    let held: Vec<usize> = lens
        .iter()
        .map(|&id| canvases.path_len(id as u32).unwrap())
        .collect();
    assert_eq!(held, [1 << 20, 1 << 20, 1 << 20, 1 << 20, 0]);
    // The five, and as many more as the bound holds.
    let made = (0..MAX_CANVAS_OBJECTS)
        .take_while(|_| canvases.call(call("path", &[], &[]), None).is_ok())
        .count();
    assert_eq!(made + 5, MAX_CANVAS_OBJECTS);
    let error = canvases.call(call("path", &[], &[]), None).unwrap_err();
    assert_eq!(error.name, "RangeError");
    // Letting one go makes room for one.
    let id = lens[0];
    canvases
        .call(call("release", &[2.0, id], &[]), None)
        .unwrap();
    assert!(canvases.call(call("path", &[], &[]), None).is_ok());
    assert!(canvases.call(call("path", &[], &[]), None).is_err());
}

/// The text the layout shows as the element `node`'s own, laid out at
/// `width`.
fn shown_text(tab: &mut Tab, node: dom::NodeId, width: f32) -> String {
    fn texts(fragments: &[layout::Fragment], node: dom::NodeId, out: &mut String) {
        for fragment in fragments {
            if let layout::FragmentKind::Text(text) = &fragment.kind
                && fragment.node == node
            {
                out.push_str(text);
            }
            texts(&fragment.children, node, out);
        }
    }
    let mut out = String::new();
    texts(&tab.engine.layout(width).roots, node, &mut out);
    out
}

/// A frame's pixel at (x, y), the frame `width` wide.
fn pixel(frame: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
}

/// A tab showing `page` with scripts on or off, loaded; and the server,
/// which must outlive it.
fn shown(page: &'static str, scripting: bool) -> (net::test_server::TestServer, Tab) {
    let server = serve(move |path| match path {
        "/page.html" => Some(html(page)),
        _ => None,
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(scripting);
    let url = server.url("/page.html");
    tab.engine.navigate(url.clone());
    tab.wait_until(|engine| {
        incoming(engine).is_none()
            && engine.url() == Some(&url)
            && engine.load_state() == &LoadState::Loaded
    });
    (server, tab)
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

#[test]
fn a_canvas_shows_its_bitmap_and_its_changes_without_a_layout() {
    let (_server, mut tab) = shown(
        "<!DOCTYPE html><style>body { margin: 0 } canvas { display: block }</style>\
         <canvas id=c width=40 height=20>fallback</canvas><script>\
         const x = document.getElementById('c').getContext('2d'); x.fillStyle = '#f00'; x.fillRect(0, 0, 10, 10);\
         setTimeout(() => { x.fillStyle = '#00f'; x.fillRect(0, 0, 10, 10); }, 0);\
         </script>",
        true,
    );
    let frame = tab.engine.frame(100, 60);
    assert_eq!(
        [
            pixel(&frame, 100, 5, 5),
            pixel(&frame, 100, 15, 5),
            pixel(&frame, 100, 5, 30)
        ],
        [RED, WHITE, WHITE]
    );
    let c = by_id(&tab.engine, "c");
    assert_eq!(shown_text(&mut tab, c, 100.0), "");
    let layers = tab.engine.layers();
    assert!(
        layers
            .iter()
            .any(|l| l.key == paint::GroupKey::Canvas(c) && l.reason == paint::LayerReason::Canvas),
        "{layers:?}"
    );
    // The timer draws again: a frame is wanted, which shows it, and
    // nothing is laid out for it.
    let laid_out = tab.engine.boxes_laid_out();
    let deadline = std::time::Instant::now() + WAIT;
    while !tab.engine.needs_frame() {
        assert!(std::time::Instant::now() < deadline, "no frame wanted");
        tab.engine.run_tasks();
    }
    let frame = tab.engine.frame(100, 60);
    assert_eq!(pixel(&frame, 100, 5, 5), BLUE);
    assert_eq!(tab.engine.boxes_laid_out(), laid_out);
    assert!(!tab.engine.needs_frame());
}

#[test]
fn a_canvas_is_sized_by_its_attributes_and_scaled_by_its_style() {
    let (_server, mut tab) = shown(
        "<!DOCTYPE html><style>body { margin: 0 } canvas { display: block }\
         #b { width: 80px }</style>\
         <canvas id=a></canvas><canvas id=b width=40 height=20></canvas><script>\
         const x = document.getElementById('b').getContext('2d'); x.fillStyle = '#f00'; x.fillRect(0, 0, 10, 10);\
         </script>",
        true,
    );
    let frame = tab.engine.frame(400, 400);
    // 300 by 150 by default, transparent; then 40 by 20 drawn at twice
    // its size, keeping its ratio.
    assert_eq!(pixel(&frame, 400, 299, 149), WHITE);
    assert_eq!(pixel(&frame, 400, 15, 165), RED);
    assert_eq!(pixel(&frame, 400, 25, 165), WHITE);
    assert_eq!(pixel(&frame, 400, 15, 190), WHITE);
    let b = by_id(&tab.engine, "b");
    let a = by_id(&tab.engine, "a");
    let mut size = |node| {
        let layout = tab.engine.layout(400.0);
        crate::fragment_of(&layout.roots, node).map(|f| (f.w, f.h))
    };
    assert_eq!(size(a), Some((300.0, 150.0)));
    assert_eq!(size(b), Some((80.0, 40.0)));
}

#[test]
fn a_canvas_without_scripts_shows_its_fallback_content() {
    let (_server, mut tab) = shown(
        "<!DOCTYPE html><canvas id=c width=40 height=20>fallback</canvas>",
        false,
    );
    tab.engine.frame(100, 60);
    let c = by_id(&tab.engine, "c");
    assert_eq!(shown_text(&mut tab, c, 100.0), "fallback");
}

#[test]
fn a_printed_canvas_shows_its_bitmap() {
    let (_server, tab) = shown(
        "<!DOCTYPE html><canvas id=c width=40 height=20></canvas><script>\
         const x = document.getElementById('c').getContext('2d');\
         x.fillStyle = '#f00'; x.fillRect(0, 0, 40, 20);\
         </script>",
        true,
    );
    let pages = tab
        .engine
        .printed_pages(&crate::PrintOptions::default())
        .expect("printed");
    let canvas = pages[0]
        .list
        .items
        .iter()
        .find_map(|item| match item {
            paint::DisplayItem::Image { image, .. } if paint::canvas_node(image.id).is_some() => {
                image.pixels.clone()
            }
            _ => None,
        })
        .expect("the canvas's pixels");
    assert_eq!(&canvas.frames[0].pixels[..4], &RED);
}

#[test]
fn text_attributes_read_back_as_the_reference_browser_reads_them() {
    let lines = run(r##"
step("attrs", () => { const x = fresh().getContext("2d"); const r = [];
  for (const [p, vs] of [["textAlign", ["left", "right", "center", "start", "end", "bogus", "LEFT"]], ["textBaseline", ["top", "hanging", "middle", "alphabetic", "ideographic", "bottom", "bogus"]],
    ["direction", ["rtl", "ltr", "inherit", "bogus"]], ["fontKerning", ["normal", "none", "auto", "bogus"]], ["fontStretch", ["condensed", "ultra-expanded", "50%", "bogus", "normal"]],
    ["fontVariantCaps", ["small-caps", "all-small-caps", "petite-caps", "unicase", "titling-caps", "bogus", "normal"]],
    ["textRendering", ["optimizeSpeed", "optimizelegibility", "geometricPrecision", "bogus", "auto"]], ["lang", ["en", "", "bogus-lang", "inherit"]]]) {
    for (const v of vs) { x[p] = v; r.push(x[p]); } } return r.join(" "); });
step("spacing", () => { const x = fresh().getContext("2d"); const r = []; for (const v of ["2px", "0.5em", "1pt", "-3px", "bogus", "2", "10%", "1rem", "0", "  4px "]) {
  x.letterSpacing = v; r.push(x.letterSpacing); x.wordSpacing = v; r.push(x.wordSpacing); } return r.join(" "); });
step("fontResetsStretch", () => { const x = fresh().getContext("2d"); x.fontStretch = "condensed"; x.fontVariantCaps = "small-caps"; x.fontKerning = "none"; x.font = "12px serif";
  return [x.fontStretch, x.fontVariantCaps, x.fontKerning, x.font]; });
step("stretchInFont", () => { const x = fresh().getContext("2d"); x.font = "condensed 12px serif"; return [x.font, x.fontStretch, x.fontVariantCaps]; });
step("fonts", () => { const x = fresh().getContext("2d"); const r = []; for (const f of ["italic small-caps 300 16px/2 Georgia, serif", "1em sans-serif", "bogus", "12pt monospace"]) {
  x.font = "10px sans-serif"; x.font = f; r.push(x.font); } return r; });
step("relative", () => { const d = document.createElement("div"); d.style.fontSize = "20px"; document.body.append(d); const c = fresh(); d.append(c);
  const x = c.getContext("2d"); const r = []; x.font = "1em serif"; r.push(x.font); x.font = "50% serif"; r.push(x.font); x.font = "larger serif"; r.push(x.font);
  x.font = "2rem serif"; r.push(x.font); return r; });
step("metricsProto", () => own(TextMetrics.prototype));
step("defaults", () => { const x = fresh().getContext("2d"); return [x.font, x.textAlign, x.textBaseline, x.direction, x.letterSpacing, x.wordSpacing, x.fontKerning, x.fontStretch, x.fontVariantCaps, x.textRendering, x.lang]; });
step("textErrors", () => { const x = fresh().getContext("2d"); const r = []; try { x.fillText("a"); } catch (e) { r.push(err(e)); } try { x.measureText(); } catch (e) { r.push(err(e)); }
  try { x.strokeText("a", 1); } catch (e) { r.push(err(e)); } x.fillText("a", 0, 10, -1); x.fillText("a", 0, 10, NaN); x.fillText("a", 0, 10, 0); r.push("ok"); return r; });
step("drawn", () => { const x = fresh(100, 40).getContext("2d"); x.font = "30px monospace"; const count = () => { let n = 0;
  const d = x.getImageData(0, 0, 100, 40).data; for (let i = 3; i < d.length; i += 4) if (d[i] > 0) n++; return n; };
  x.fillText("WW", 5, 30); const a = count(); x.clearRect(0, 0, 100, 40); x.fillText("WW", 5, 30, 10); const b = count(); x.clearRect(0, 0, 100, 40);
  x.fillText("WW", 5, 30, -1); const c = count(); x.clearRect(0, 0, 100, 40); x.strokeText("WW", 5, 30); const d = count(); return [a > 100, b < a / 2, c, d > 50]; });
step("metrics", () => { const x = fresh().getContext("2d"); x.font = "20px monospace"; const m = x.measureText("Hello");
  return [m.width > 40, m.width === x.measureText("Hello").width, m.alphabeticBaseline, m.ideographicBaseline === -m.fontBoundingBoxDescent, typeof m.emHeightAscent]; });
step("rtl", () => { const x = fresh().getContext("2d"); x.font = "20px monospace"; x.direction = "rtl"; const m = x.measureText("ab"); return [m.actualBoundingBoxLeft > 10, m.actualBoundingBoxRight < 2, x.textAlign]; });
step("direction", () => { const c = fresh(); c.dir = "rtl"; const x = c.getContext("2d"); return [x.direction]; });
"##);
    check(
        &lines,
        &[
            (
                "attrs",
                "\"left right center start end end end top hanging middle alphabetic ideographic \
                 bottom bottom rtl ltr ltr ltr normal none auto auto condensed ultra-expanded \
                 ultra-expanded ultra-expanded normal small-caps all-small-caps petite-caps \
                 unicase titling-caps titling-caps normal optimizeSpeed optimizeSpeed \
                 geometricPrecision geometricPrecision auto en  bogus-lang inherit\"",
            ),
            (
                "spacing",
                "\"2px 2px 0.5em 0.5em 1pt 1pt -3px -3px -3px -3px -3px -3px -3px -3px 1rem 1rem \
                 1rem 1rem 1rem 1rem\"",
            ),
            (
                "fontResetsStretch",
                "[\"normal\",\"normal\",\"none\",\"12px serif\"]",
            ),
            ("stretchInFont", "[\"12px serif\",\"condensed\",\"normal\"]"),
            (
                "fonts",
                "[\"italic 300 small-caps 16px Georgia, serif\",\"10px sans-serif\",\
                 \"10px sans-serif\",\"16px monospace\"]",
            ),
            (
                "relative",
                "[\"20px serif\",\"10px serif\",\"24px serif\",\"32px serif\"]",
            ),
            (
                "metricsProto",
                "\"actualBoundingBoxAscent actualBoundingBoxDescent actualBoundingBoxLeft \
                 actualBoundingBoxRight alphabeticBaseline constructor fontBoundingBoxAscent \
                 fontBoundingBoxDescent hangingBaseline ideographicBaseline width\"",
            ),
            (
                "defaults",
                "[\"10px sans-serif\",\"start\",\"alphabetic\",\"ltr\",\"0px\",\"0px\",\"auto\",\
                 \"normal\",\"normal\",\"auto\",\"inherit\"]",
            ),
            (
                "textErrors",
                "[\"threw TypeError: Failed to execute 'fillText' on 'CanvasRenderingContext2D': \
                 3 arguments required, but only 1 present.\",\"threw TypeError: Failed to execute \
                 'measureText' on 'CanvasRenderingContext2D': 1 argument required, but only 0 \
                 present.\",\"threw TypeError: Failed to execute 'strokeText' on \
                 'CanvasRenderingContext2D': 3 arguments required, but only 2 present.\",\"ok\"]",
            ),
            ("drawn", "[true,true,0,true]"),
            ("metrics", "[true,true,0,true,\"undefined\"]"),
            ("rtl", "[true,true,\"start\"]"),
            ("direction", "[\"rtl\"]"),
        ],
    );
}

/// The probe's image, a PNG two by two: red and magenta over blue and
/// white.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 2, 0,
    0, 0, 253, 212, 154, 115, 0, 0, 0, 18, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240, 31, 8,
    129, 36, 16, 0, 0, 40, 229, 6, 250, 245, 50, 76, 156, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
    130,
];

/// The lines a page's `script` logs, with `other` the URL of another
/// origin's server: each serves `/img.png`, and `/acao.png` with
/// `Access-Control-Allow-Origin: *`. The page logs `done` last.
fn run_with_images(script: &str) -> Vec<String> {
    let images = |path: &str| match path.split('?').next().unwrap_or_default() {
        "/img.png" => Some(net::test_server::Reply::ok("image/png", PNG)),
        "/acao.png" => Some(net::test_server::Reply::with_fields(
            200,
            &[
                ("Content-Type", "image/png"),
                ("Access-Control-Allow-Origin", "*"),
            ],
            PNG,
        )),
        _ => None,
    };
    let other_server = serve(images);
    let other = other_server
        .url("/")
        .as_str()
        .trim_end_matches('/')
        .to_owned();
    let page = format!(
        "<!DOCTYPE html><script>\n\
         const other = '{other}';\n\
         const err = (e) => 'threw ' + (e && e.name) + ': ' + (e && e.message);\n\
         const px = (ctx, x, y) => Array.from(ctx.getImageData(x, y, 1, 1).data);\n\
         const fresh = (w, h) => {{ const c = document.createElement('canvas'); \
         c.width = w === undefined ? 10 : w; c.height = h === undefined ? 10 : h; return c; }};\n\
         const load = (src, cors) => new Promise((ok) => {{ const i = new Image(); \
         if (cors !== undefined) i.crossOrigin = cors; i.onload = () => ok(i); \
         i.onerror = () => ok(i); i.src = src; }});\n\
         const step = (name, f) => {{ let v; try {{ v = JSON.stringify(f()); }} \
         catch (e) {{ v = err(e); }} console.log(name + '=' + v); }};\n\
         (async () => {{\n{script}\nconsole.log('done');\n}})();</script>"
    );
    let server = serve(move |path| match path {
        "/page.html" => Some(html(&page)),
        path => images(path),
    });
    let mut tab = Tab::new();
    tab.engine.set_scripting(true);
    let url = server.url("/page.html");
    tab.engine.navigate(url.clone());
    let mut lines = Vec::new();
    let deadline = std::time::Instant::now() + WAIT;
    while !lines.iter().any(|line: &String| line == "done") {
        assert!(std::time::Instant::now() < deadline, "only {lines:#?}");
        tab.engine.poll_network();
        tab.engine.run_tasks();
        lines.extend(
            tab.engine
                .take_console()
                .into_iter()
                .map(|entry| entry.text),
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    lines
}

#[test]
fn images_are_drawn_and_their_origins_taint_as_the_reference_browser_says() {
    let lines = run_with_images(
        r##"
const probe = (img) => { if (!img.complete || img.naturalWidth === 0) return "load error";
  const c = fresh(2, 2); const x = c.getContext("2d"); x.drawImage(img, 0, 0); const r = [img.naturalWidth];
  try { r.push(px(x, 0, 0)); } catch (e) { r.push(err(e)); }
  try { x.createPattern(img, "repeat"); r.push("pattern ok"); } catch (e) { r.push(err(e)); } return r; };
const same = await load("/img.png");
step("same", () => probe(same));
const cross = await load(other + "/img.png");
step("cross", () => probe(cross));
const anon = await load(other + "/acao.png", "anonymous");
step("crossAnon", () => probe(anon));
const refused = await load(other + "/img.png?no-acao", "anonymous");
step("crossAnonNoAcao", () => probe(refused));
// The URL fetched without CORS above, asked for again with it: a fetch
// of its own, which the other origin does not allow.
const again = await load(other + "/img.png", "anonymous");
step("sameUrlInCors", () => probe(again));
const c = fresh(2, 2); const x = c.getContext("2d"); x.drawImage(cross, 0, 0);
const d = fresh(2, 2).getContext("2d"); d.drawImage(c, 0, 0);
step("propagates", () => { try { d.getImageData(0, 0, 1, 1); return "no"; } catch (e) { return err(e); } });
step("patternTaints", () => { const y = fresh(2, 2).getContext("2d"); y.fillStyle = y.createPattern(cross, "repeat"); const before = px(y, 0, 0);
  y.fillRect(0, 0, 2, 2); try { y.getImageData(0, 0, 1, 1); return "no"; } catch (e) { return [before, err(e)]; } });
c.width = 2;
step("resetClears", () => { try { x.getImageData(0, 0, 1, 1); return "clean again"; } catch (e) { return err(e); } });
"##,
    );
    let tainted = "threw SecurityError: Failed to execute 'getImageData' on \
                   'CanvasRenderingContext2D': The canvas has been tainted by cross-origin data.";
    let want = [
        "same=[2,[255,0,0,255],\"pattern ok\"]".to_owned(),
        format!("cross=[2,{},\"pattern ok\"]", serde_text(tainted)),
        "crossAnon=[2,[255,0,0,255],\"pattern ok\"]".to_owned(),
        "crossAnonNoAcao=\"load error\"".to_owned(),
        "sameUrlInCors=\"load error\"".to_owned(),
        format!("propagates={}", serde_text(tainted)),
        format!("patternTaints=[[0,0,0,0],{}]", serde_text(tainted)),
        "resetClears=\"clean again\"".to_owned(),
    ];
    for line in want {
        assert!(lines.contains(&line), "no {line} in {lines:#?}");
    }
}

#[test]
fn settings_names_and_sizes_convert_as_web_idl_says() {
    // As the reference browser gave them (the M30 notes, p8-wpt.html,
    // and WPT's 2d.getContext.options).
    let lines = run(r#"
const read = [];
const options = { get alpha() { read.push("alpha"); return true; },
  get willReadFrequently() { read.push("willReadFrequently"); return false; },
  get desynchronized() { read.push("desynchronized"); return false; },
  get colorSpace() { read.push("colorSpace");
    return { toString() { read.push("colorSpace toString"); return "srgb"; } }; } };
fresh().getContext("2d", options);
step("order", () => read);
step("offscreen2D", () => new OffscreenCanvas(1, 1).getContext("2D"));
step("offscreenWebgl", () => { const gl = new OffscreenCanvas(1, 1).getContext("webgl"); return gl === null || gl instanceof WebGLRenderingContext; });
step("element2D", () => fresh().getContext("2D"));
step("number", () => fresh().getContext("2d", 5) !== null);
step("u8", () => new ImageData(new Uint8Array(8), 1, 2));
step("window", () => new ImageData(self, 4, 4));
step("windowTwo", () => new ImageData(self, 4));
step("zeroSettings", () => new ImageData(0, 4, 4));
"#);
    let settings = "threw TypeError: Failed to execute 'constructor' on 'ImageData': \
                    The provided value is not of type 'ImageDataSettings'.";
    check(
        &lines,
        &[
            (
                "order",
                r#"["alpha","colorSpace","colorSpace toString","desynchronized","willReadFrequently"]"#,
            ),
            (
                "offscreen2D",
                "threw TypeError: Failed to execute 'getContext' on 'OffscreenCanvas': The \
                 provided value '2D' is not a valid enum value of type \
                 OffscreenRenderingContextId.",
            ),
            ("offscreenWebgl", "true"),
            ("element2D", "null"),
            ("number", "true"),
            ("u8", settings),
            ("window", settings),
            (
                "windowTwo",
                "threw IndexSizeError: Failed to construct 'ImageData': The source width is \
                 zero or not a number.",
            ),
            ("zeroSettings", settings),
        ],
    );
}

#[test]
fn a_canvas_without_a_context_draws_as_transparent_black() {
    // WPT's 2d.composite.uncovered.nocontext and 2d.pattern.basic.nocontext.
    let lines = run(r##"
const x = fresh(4, 4).getContext("2d");
x.fillStyle = "#0f0"; x.fillRect(0, 0, 4, 4);
x.globalCompositeOperation = "copy";
x.drawImage(fresh(4, 4), 0, 0);
step("copied", () => px(x, 1, 1));
const y = fresh(4, 4).getContext("2d");
y.fillStyle = "#0f0"; y.fillRect(0, 0, 4, 4);
const pattern = y.createPattern(fresh(4, 4), "no-repeat");
step("pattern", () => pattern !== null);
y.fillStyle = pattern; y.fillRect(0, 0, 4, 4);
step("patterned", () => px(y, 1, 1));
const z = fresh(4, 4).getContext("2d");
z.globalCompositeOperation = "copy";
z.fillRect(0, 0, 4, 4);
z.drawImage(new OffscreenCanvas(4, 4), 0, 0);
step("offscreen", () => px(z, 1, 1));
"##);
    check(
        &lines,
        &[
            ("copied", "[0,0,0,0]"),
            ("pattern", "true"),
            ("patterned", "[0,255,0,255]"),
            ("offscreen", "[0,0,0,0]"),
        ],
    );
}

#[test]
fn an_image_whose_source_changed_is_not_drawn_until_it_comes() {
    // WPT's 2d.drawImage.incomplete and 2d.pattern.image.incomplete: an
    // image asked for again, or no longer, is not yet there to draw.
    let lines = run_with_images(
        r##"
const img = await load("/img.png");
const x = fresh(2, 2).getContext("2d");
x.fillStyle = "#0f0";
const probe = () => { x.fillRect(0, 0, 2, 2); x.drawImage(img, 0, 0);
  return [img.complete, px(x, 0, 0), x.createPattern(img, "repeat") === null]; };
step("loaded", probe);
img.src = "/img.png?again";
step("changed", probe);
img.removeAttribute("src");
step("removed", probe);
img.src = "";
step("empty", probe);
"##,
    );
    check(
        &lines,
        &[
            ("loaded", "[true,[255,0,0,255],false]"),
            ("changed", "[false,[0,255,0,255],true]"),
            ("removed", "[true,[0,255,0,255],true]"),
            ("empty", "[true,[0,255,0,255],true]"),
        ],
    );
}

#[test]
fn colours_kept_parsed_read_back_as_given_but_currentcolor_as_it_is_now() {
    // M30.4's cache of the colours the styles were given: the same text
    // again is the same colour, a text that does not parse still changes
    // nothing, and currentcolor follows the element's colour.
    let lines = run(r##"
const x = fresh().getContext("2d");
x.fillStyle = "hsl(120, 100%, 25%)";
x.fillStyle = "#000";
x.fillStyle = "hsl(120, 100%, 25%)";
step("again", () => x.fillStyle);
x.fillStyle = "bogus";
x.fillStyle = "bogus";
step("bogus", () => x.fillStyle);
for (let i = 0; i < 600; i++) { x.fillStyle = "rgb(" + (i % 256) + ", 0, 0)"; }
step("many", () => x.fillStyle);
const c = fresh();
document.body.append(c);
c.style.color = "red";
const y = c.getContext("2d");
y.fillStyle = "currentcolor";
const first = y.fillStyle;
c.style.color = "blue";
y.fillStyle = "#fff";
y.fillStyle = "currentcolor";
step("current", () => [first, y.fillStyle]);
"##);
    check(
        &lines,
        &[
            ("again", "\"#008000\""),
            ("bogus", "\"#008000\""),
            ("many", "\"#570000\""),
            ("current", r##"["#ff0000","#0000ff"]"##),
        ],
    );
}

#[test]
fn the_colours_kept_parsed_are_bounded() {
    let canvases = Canvases::default();
    let black = canvas::Rgba::BLACK;
    for i in 0..MAX_KEPT_COLOURS * 3 {
        let text = format!("rgb({}, {}, 0)", i % 256, i / 256);
        assert!(canvases.colour(&text, black).is_some());
        assert!(canvases.kept_colours() <= MAX_KEPT_COLOURS);
    }
    assert_eq!(
        canvases.colour("rgb(1, 2, 3)", black),
        Some(canvas::Rgba::new(1, 2, 3, 255))
    );
}

#[test]
fn arc_to_through_the_bindings_turns_toward_its_second_point() {
    // A corner at (20, 0) rounded toward (20, 30): the path's inside is
    // the rectangle's but for the corner's outside.
    let lines = run(r##"
const x = fresh(40, 40).getContext("2d");
x.moveTo(0, 0);
x.arcTo(20, 0, 20, 30, 8);
x.lineTo(20, 20);
x.lineTo(0, 20);
x.closePath();
step("inside", () => [[10, 10], [17, 3], [19, 1], [10, 1], [22, 10], [21, 6]].map(([a, b]) => x.isPointInPath(a, b)));
"##);
    check(&lines, &[("inside", "[true,true,false,true,false,false]")]);
}

/// `text` as JSON writes a string.
fn serde_text(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[test]
fn draw_image_and_create_pattern_answer_as_the_reference_browser_does() {
    let lines = run_with_images(
        r##"
const img = await load("/img.png");
const broken = await load("/missing.png");
step("forms", () => { const x = fresh(10, 10).getContext("2d"); x.imageSmoothingEnabled = false; x.drawImage(img, 0, 0); const a = [px(x, 0, 0), px(x, 1, 0), px(x, 0, 1), px(x, 1, 1), px(x, 2, 2)];
  x.clearRect(0, 0, 10, 10); x.drawImage(img, 2, 2, 4, 4); const b = [px(x, 2, 2), px(x, 5, 2), px(x, 2, 5), px(x, 5, 5), px(x, 6, 6)];
  x.clearRect(0, 0, 10, 10); x.drawImage(img, 1, 0, 1, 2, 0, 0, 2, 4); const c = [px(x, 0, 0), px(x, 1, 3), px(x, 2, 0)];
  return [a, b, c]; });
step("broken", () => { const x = fresh().getContext("2d"); try { x.drawImage(broken, 0, 0); return "drew"; } catch (e) { return err(e); } });
step("brokenPattern", () => { const x = fresh().getContext("2d"); try { return String(x.createPattern(broken, "repeat")); } catch (e) { return err(e); } });
step("unloadedPattern", () => { const x = fresh().getContext("2d"); return String(x.createPattern(new Image(), "repeat")); });
step("emptyCanvas", () => { const x = fresh().getContext("2d"); const r = []; try { x.drawImage(fresh(0, 0), 0, 0); r.push("drew"); } catch (e) { r.push(err(e)); }
  try { r.push(String(x.createPattern(fresh(0, 5), "repeat"))); } catch (e) { r.push(err(e)); } return r; });
step("noContext", () => { const x = fresh().getContext("2d"); x.fillRect(0, 0, 10, 10); x.drawImage(fresh(4, 4), 0, 0); return px(x, 1, 1); });
step("self", () => { const c = fresh(4, 4); const x = c.getContext("2d"); x.fillStyle = "#f00"; x.fillRect(0, 0, 2, 2); x.drawImage(c, 2, 2); return [px(x, 3, 3), px(x, 1, 1)]; });
step("patternForms", () => { const x = fresh(8, 8).getContext("2d"); const r = []; for (const rep of ["repeat", "repeat-x", "repeat-y", "no-repeat", "", null]) {
  x.clearRect(0, 0, 8, 8); const p = x.createPattern(img, rep); x.fillStyle = p; x.fillRect(0, 0, 8, 8); r.push(String(rep) + ":" + [px(x, 2, 0)[3], px(x, 0, 2)[3], px(x, 3, 3)[3]].join(",")); } return r; });
step("patternTransform", () => { const x = fresh(8, 8).getContext("2d"); const p = x.createPattern(img, "no-repeat"); p.setTransform(new DOMMatrix([1, 0, 0, 1, 3, 3])); x.fillStyle = p; x.fillRect(0, 0, 8, 8);
  return [px(x, 3, 3), px(x, 0, 0), px(x, 4, 4)]; });
step("patternErrors", () => { const x = fresh().getContext("2d"); const r = []; try { x.createPattern({}, "repeat"); } catch (e) { r.push(err(e)); }
  try { x.createPattern(img); } catch (e) { r.push(err(e)); } try { r.push(String(x.createPattern(img, undefined))); } catch (e) { r.push(err(e)); } return r; });
step("argCounts", () => { const x = fresh().getContext("2d"); const r = []; for (const n of [1, 2, 4, 6, 7, 8]) { try { x.drawImage.apply(x, [img].concat(new Array(n - 1).fill(0))); r.push(n + ":ok"); } catch (e) { r.push(n + ":" + err(e)); } } return r; });
step("drawImageErrors", () => { const x = fresh().getContext("2d"); try { x.drawImage({}, 0, 0); } catch (e) { return err(e); } });
const fresh_image = new Image(); const none = fresh_image.complete; fresh_image.src = "/img.png?complete";
const coming = fresh_image.complete; await new Promise((ok) => { fresh_image.onload = ok; });
step("complete", () => [none, coming, fresh_image.complete, fresh_image.naturalWidth, fresh_image.naturalHeight, fresh_image.width, fresh_image.height]);
step("crossOrigin", () => { const i = new Image(5, 6); const r = [i.crossOrigin, i.width, i.height]; for (const v of ["", "USE-CREDENTIALS", "bogus"]) { i.crossOrigin = v; r.push(i.crossOrigin, i.getAttribute("crossorigin")); }
  i.crossOrigin = null; r.push(i.crossOrigin, i.hasAttribute("crossorigin")); return r; });
step("detached", () => { const x = fresh(4, 4).getContext("2d"); x.drawImage(img, 0, 0, 4, 4); return [img.isConnected, px(x, 0, 0)[3]]; });
"##,
    );
    let want = [
        "forms=[[[255,0,0,255],[255,0,255,255],[0,0,255,255],[255,255,255,255],[0,0,0,0]],\
         [[255,0,0,255],[255,0,255,255],[0,0,255,255],[255,255,255,255],[0,0,0,0]],\
         [[255,0,255,255],[255,255,255,255],[0,0,0,0]]]",
        "broken=\"threw InvalidStateError: Failed to execute 'drawImage' on \
         'CanvasRenderingContext2D': The HTMLImageElement provided is in the 'broken' state.\"",
        "brokenPattern=\"threw InvalidStateError: Failed to execute 'createPattern' on \
         'CanvasRenderingContext2D': Source image is in the 'broken' state.\"",
        "unloadedPattern=\"null\"",
        "emptyCanvas=[\"threw InvalidStateError: Failed to execute 'drawImage' on \
         'CanvasRenderingContext2D': The image argument is a canvas element with a width or \
         height of 0.\",\"threw InvalidStateError: Failed to execute 'createPattern' on \
         'CanvasRenderingContext2D': The image argument is a canvas element with a width or \
         height of 0.\"]",
        "noContext=[0,0,0,255]",
        "self=[[255,0,0,255],[255,0,0,255]]",
        "patternForms=[\"repeat:255,255,255\",\"repeat-x:255,0,0\",\"repeat-y:0,255,0\",\
         \"no-repeat:0,0,0\",\":255,255,255\",\"null:255,255,255\"]",
        "patternTransform=[[255,0,0,255],[0,0,0,0],[255,255,255,255]]",
        "patternErrors=[\"threw TypeError: Failed to execute 'createPattern' on \
         'CanvasRenderingContext2D': The provided value is not of type '(CSSImageValue or \
         HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or ImageBitmap or \
         OffscreenCanvas or SVGImageElement or VideoFrame)'.\",\"threw TypeError: Failed to \
         execute 'createPattern' on 'CanvasRenderingContext2D': 2 arguments required, but only \
         1 present.\",\"threw SyntaxError: Failed to execute 'createPattern' on \
         'CanvasRenderingContext2D': The provided type ('undefined') is not one of 'repeat', \
         'no-repeat', 'repeat-x', or 'repeat-y'.\"]",
        "argCounts=[\"1:threw TypeError: Failed to execute 'drawImage' on \
         'CanvasRenderingContext2D': 3 arguments required, but only 1 present.\",\"2:threw \
         TypeError: Failed to execute 'drawImage' on 'CanvasRenderingContext2D': 3 arguments \
         required, but only 2 present.\",\"4:threw TypeError: Failed to execute 'drawImage' on \
         'CanvasRenderingContext2D': Overload resolution failed.\",\"6:threw TypeError: Failed \
         to execute 'drawImage' on 'CanvasRenderingContext2D': Overload resolution failed.\",\
         \"7:threw TypeError: Failed to execute 'drawImage' on 'CanvasRenderingContext2D': \
         Overload resolution failed.\",\"8:threw TypeError: Failed to execute 'drawImage' on \
         'CanvasRenderingContext2D': Overload resolution failed.\"]",
        "drawImageErrors=\"threw TypeError: Failed to execute 'drawImage' on \
         'CanvasRenderingContext2D': The provided value is not of type '(CSSImageValue or \
         HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or ImageBitmap or \
         OffscreenCanvas or SVGImageElement or VideoFrame)'.\"",
        "complete=[true,false,true,2,2,2,2]",
        "crossOrigin=[null,5,6,\"anonymous\",\"\",\"use-credentials\",\"USE-CREDENTIALS\",\
         \"anonymous\",\"bogus\",null,false]",
        // `new Image()`, never in the document, loads and draws.
        "detached=[false,255]",
    ];
    for line in want {
        assert!(lines.iter().any(|l| l == line), "no {line} in {lines:#?}");
    }
}

#[test]
fn bitmaps_are_exported_and_offscreen_canvases_draw_as_the_reference_browser_says() {
    let lines = run_with_images(
        r##"
const img = await load("/img.png");
const cross = await load(other + "/img.png");
const own = (p) => Object.getOwnPropertyNames(p).sort().join(" ");
const stepAsync = async (name, f) => { let v; try { v = JSON.stringify(await f()); } catch (e) { v = err(e); } console.log(name + "=" + v); };
step("protoOffscreen", () => own(OffscreenCanvas.prototype));
step("protoOffscreenCtx", () => own(OffscreenCanvasRenderingContext2D.prototype) === own(CanvasRenderingContext2D.prototype).replace(" drawFocusIfNeeded", ""));
step("protoBitmap", () => own(ImageBitmap.prototype));
step("dataUrl", () => { const c = fresh(2, 2); const x = c.getContext("2d"); x.fillStyle = "#f00"; x.fillRect(0, 0, 1, 1);
  return [c.toDataURL().slice(0, 30), c.toDataURL("bogus").slice(0, 22), c.toDataURL("IMAGE/PNG").slice(0, 22), fresh(0, 3).toDataURL(), fresh(3, 0).toDataURL("image/jpeg")]; });
step("dataUrlNoContext", () => fresh(2, 2).toDataURL().slice(0, 22));
step("dataUrlTainted", () => { const c = fresh(2, 2); c.getContext("2d").drawImage(cross, 0, 0); const r = [];
  try { c.toDataURL(); } catch (e) { r.push(err(e)); } try { c.toBlob(() => {}); } catch (e) { r.push(err(e)); } return r; });
await stepAsync("toBlob", () => new Promise((ok) => { const c = fresh(2, 2); c.getContext("2d").fillRect(0, 0, 1, 1); let sync = true;
  c.toBlob((b) => { ok([sync, String(b), b.type, b.size > 50]); }); sync = false; }));
await stepAsync("toBlobEmpty", () => new Promise((ok) => { fresh(0, 2).toBlob((b) => ok(String(b))); }));
await stepAsync("toBlobType", () => new Promise((ok) => { fresh(2, 2).toBlob((b) => ok(b.type), "image/bogus"); }));
step("toBlobErrors", () => { const r = []; try { fresh().toBlob(); } catch (e) { r.push(err(e)); } try { fresh().toBlob(5); } catch (e) { r.push(err(e)); } return r; });
await stepAsync("pngRoundTrip", async () => { const c = fresh(2, 1); const x = c.getContext("2d"); x.fillStyle = "rgba(255, 0, 0, 0.5)"; x.fillRect(0, 0, 1, 1);
  const blob = await new Promise((ok) => c.toBlob(ok)); const b = await createImageBitmap(blob); const d = fresh(2, 1).getContext("2d"); d.drawImage(b, 0, 0); return [px(d, 0, 0), px(d, 1, 0)]; });
step("offscreen", () => { const o = new OffscreenCanvas(3, 2); const r = [o.width, o.height, String(o)]; const x = o.getContext("2d"); r.push(String(x), x === o.getContext("2d"), x.canvas === o);
  x.fillStyle = "#0f0"; x.fillRect(0, 0, 1, 1); r.push(px(x, 0, 0)); o.width = 4; r.push(px(x, 0, 0), o.width, o.getContext("webgl"));
  try { new OffscreenCanvas(); } catch (e) { r.push(err(e)); } try { OffscreenCanvas(1, 1); r.push("called"); } catch (e) { r.push(e.name); }
  try { new OffscreenCanvas(-1, 2); } catch (e) { r.push(err(e)); } return r; });
step("offscreenDraw", () => { const o = new OffscreenCanvas(2, 2); const x = o.getContext("2d"); x.fillStyle = "#00f"; x.fillRect(0, 0, 2, 2);
  const d = fresh(4, 4).getContext("2d"); d.drawImage(o, 1, 1); return [px(d, 1, 1), px(d, 0, 0)]; });
step("offscreenText", () => { const o = new OffscreenCanvas(60, 30); const x = o.getContext("2d"); x.font = "20px monospace"; x.fillText("W", 5, 20); let n = 0;
  const d = x.getImageData(0, 0, 60, 30).data; for (let i = 3; i < d.length; i += 4) if (d[i] > 0) n++; return [n > 20, x.measureText("W").width > 5]; });
await stepAsync("convertToBlob", async () => { const o = new OffscreenCanvas(2, 2); o.getContext("2d").fillRect(0, 0, 1, 1); const b = await o.convertToBlob(); return [String(b), b.type, b.size > 50]; });
await stepAsync("convertToBlobErrors", async () => { const r = []; try { await new OffscreenCanvas(0, 2).convertToBlob(); r.push("ok"); } catch (e) { r.push(err(e)); }
  const o = new OffscreenCanvas(2, 2); o.getContext("2d").drawImage(cross, 0, 0); try { await o.convertToBlob(); r.push("ok"); } catch (e) { r.push(err(e)); } return r; });
step("offscreenTaints", () => { const o = new OffscreenCanvas(2, 2); o.getContext("2d").drawImage(cross, 0, 0); const x = fresh(2, 2).getContext("2d"); x.drawImage(o, 0, 0);
  try { x.getImageData(0, 0, 1, 1); return "clean"; } catch (e) { return e.name; } });
step("transfer", () => { const o = new OffscreenCanvas(2, 2); const x = o.getContext("2d"); x.fillRect(0, 0, 2, 2); const b = o.transferToImageBitmap();
  return [String(b), b.width, b.height, px(x, 0, 0), o.width]; });
step("transferNoContext", () => { try { new OffscreenCanvas(2, 2).transferToImageBitmap(); return "ok"; } catch (e) { return err(e); } });
await stepAsync("bitmap", async () => { const b = await createImageBitmap(img); const r = [String(b), b.width, b.height]; const d = fresh(4, 4).getContext("2d"); d.imageSmoothingEnabled = false; d.drawImage(b, 0, 0);
  r.push(px(d, 1, 1)); b.close(); r.push(b.width, b.height); try { d.drawImage(b, 0, 0); r.push("drew closed"); } catch (e) { r.push(err(e)); } return r; });
await stepAsync("bitmapCrop", async () => { const b = await createImageBitmap(img, 1, 0, 1, 2); const d = fresh(4, 4).getContext("2d"); d.drawImage(b, 0, 0); return [b.width, b.height, px(d, 0, 0), px(d, 0, 1)]; });
await stepAsync("bitmapCropOutside", async () => { const b = await createImageBitmap(img, -1, -1, 3, 3); const d = fresh(4, 4).getContext("2d"); d.drawImage(b, 0, 0); return [b.width, b.height, px(d, 0, 0), px(d, 1, 1)]; });
await stepAsync("bitmapResize", async () => { const b = await createImageBitmap(img, { resizeWidth: 4, resizeHeight: 2, resizeQuality: "pixelated" }); const d = fresh(4, 4).getContext("2d"); d.drawImage(b, 0, 0);
  return [b.width, b.height, px(d, 1, 0), px(d, 2, 0)]; });
await stepAsync("bitmapResizeOneSide", async () => { const b = await createImageBitmap(img, 0, 0, 2, 1, { resizeWidth: 6 }); return [b.width, b.height]; });
await stepAsync("bitmapFlip", async () => { const b = await createImageBitmap(img, { imageOrientation: "flipY" }); const d = fresh(4, 4).getContext("2d"); d.drawImage(b, 0, 0); return [px(d, 0, 0), px(d, 0, 1)]; });
await stepAsync("bitmapSources", async () => { const r = []; const c = fresh(2, 2); c.getContext("2d").fillRect(0, 0, 1, 1); const o = new OffscreenCanvas(3, 3); o.getContext("2d");
  for (const s of [c, o, new ImageData(4, 2), await createImageBitmap(img)]) { const b = await createImageBitmap(s); r.push(b.width); }
  try { await createImageBitmap(new OffscreenCanvas(2, 2)); } catch (e) { r.push(err(e)); } return r; });
await stepAsync("bitmapErrors", async () => { const r = []; for (const f of [() => createImageBitmap(), () => createImageBitmap({}), () => createImageBitmap(img, 0, 0, 0, 1), () => createImageBitmap(img, 0, 0, 1, 0),
    () => createImageBitmap(fresh(0, 2)), () => createImageBitmap(new Image()), () => createImageBitmap(new Blob(["x"])), () => createImageBitmap(img, { resizeWidth: 0 }), () => createImageBitmap(img, 0, 0, 1)]) {
    try { await f(); r.push("ok"); } catch (e) { r.push(err(e)); } } return r; });
await stepAsync("bitmapTainted", async () => { const b = await createImageBitmap(cross); const c = fresh(2, 2); const x = c.getContext("2d"); x.drawImage(b, 0, 0); try { x.getImageData(0, 0, 1, 1); return "clean"; } catch (e) { return err(e); } });
await stepAsync("bitmapClosedErrors", async () => { const b = await createImageBitmap(img); b.close(); const r = []; try { await createImageBitmap(b); } catch (e) { r.push(err(e)); }
  try { fresh().getContext("2d").createPattern(b, "repeat"); r.push("pattern ok"); } catch (e) { r.push(err(e)); } try { fresh().getContext("2d").drawImage(b, 0, 0); r.push("drew"); } catch (e) { r.push(err(e)); } return r; });
step("tags", () => [String(new OffscreenCanvas(1, 1)), Object.prototype.toString.call(OffscreenCanvasRenderingContext2D.prototype), typeof ImageBitmap, ImageBitmap.length, OffscreenCanvas.length, createImageBitmap.length]);
step("bitmapCtor", () => { try { new ImageBitmap(); } catch (e) { return err(e); } });
"##,
    );
    let want = [
        // Chrome's but its context loss handlers (oncontextlost,
        // oncontextrestored), which no context here loses.
        "protoOffscreen=\"constructor convertToBlob getContext height transferToImageBitmap width\"",
        "protoOffscreenCtx=true",
        "protoBitmap=\"close constructor height width\"",
        "dataUrl=[\"data:image/png;base64,iVBORw0K\",\"data:image/png;base64,\",\
         \"data:image/png;base64,\",\"data:,\",\"data:,\"]",
        "dataUrlNoContext=\"data:image/png;base64,\"",
        "dataUrlTainted=[\"threw SecurityError: Failed to execute 'toDataURL' on \
         'HTMLCanvasElement': Tainted canvases may not be exported.\",\"threw SecurityError: \
         Failed to execute 'toBlob' on 'HTMLCanvasElement': Tainted canvases may not be \
         exported.\"]",
        "toBlob=[false,\"[object Blob]\",\"image/png\",true]",
        "toBlobEmpty=\"null\"",
        "toBlobType=\"image/png\"",
        "toBlobErrors=[\"threw TypeError: Failed to execute 'toBlob' on 'HTMLCanvasElement': 1 \
         argument required, but only 0 present.\",\"threw TypeError: Failed to execute 'toBlob' \
         on 'HTMLCanvasElement': parameter 1 is not of type 'Function'.\"]",
        "pngRoundTrip=[[255,0,0,128],[0,0,0,0]]",
        "offscreen=[3,2,\"[object OffscreenCanvas]\",\"[object OffscreenCanvasRenderingContext2D]\",\
         true,true,[0,255,0,255],[0,0,0,0],4,null,\"threw TypeError: Failed to construct \
         'OffscreenCanvas': 2 arguments required, but only 0 present.\",\"TypeError\",\"threw \
         TypeError: Failed to construct 'OffscreenCanvas': Value is outside the 'unsigned long' \
         value range.\"]",
        "offscreenDraw=[[0,0,255,255],[0,0,0,0]]",
        "offscreenText=[true,true]",
        "convertToBlob=[\"[object Blob]\",\"image/png\",true]",
        "convertToBlobErrors=[\"threw IndexSizeError: Failed to execute 'convertToBlob' on \
         'OffscreenCanvas': The size of the OffscreenCanvas is zero.\",\"threw SecurityError: \
         Failed to execute 'convertToBlob' on 'OffscreenCanvas': A tainted OffscreenCanvas may \
         not be exported.\"]",
        "offscreenTaints=\"SecurityError\"",
        "bitmapResizeOneSide=[6,3]",
        "transfer=[\"[object ImageBitmap]\",2,2,[0,0,0,0],2]",
        "transferNoContext=\"threw InvalidStateError: Failed to execute 'transferToImageBitmap' \
         on 'OffscreenCanvas': Cannot transfer an ImageBitmap from an OffscreenCanvas with no \
         context\"",
        "bitmap=[\"[object ImageBitmap]\",2,2,[255,255,255,255],0,0,\"threw InvalidStateError: \
         Failed to execute 'drawImage' on 'CanvasRenderingContext2D': The image source is \
         detached\"]",
        "bitmapCrop=[1,2,[255,0,255,255],[255,255,255,255]]",
        "bitmapCropOutside=[3,3,[0,0,0,0],[255,0,0,255]]",
        "bitmapResize=[4,2,[255,0,0,255],[255,0,255,255]]",
        "bitmapFlip=[[0,0,255,255],[255,0,0,255]]",
        "bitmapSources=[2,3,4,2,\"threw InvalidStateError: Failed to execute \
         'createImageBitmap' on 'Window': The ImageBitmap could not be allocated.\"]",
        "bitmapErrors=[\"threw TypeError: Failed to execute 'createImageBitmap' on 'Window': 1 \
         argument required, but only 0 present.\",\"threw TypeError: Failed to execute \
         'createImageBitmap' on 'Window': The provided value is not of type '(Blob or \
         HTMLCanvasElement or HTMLImageElement or HTMLVideoElement or ImageBitmap or ImageData or \
         OffscreenCanvas or SVGImageElement or VideoFrame)'.\",\"threw RangeError: Failed to \
         execute 'createImageBitmap' on 'Window': The crop rect width is 0.\",\"threw \
         RangeError: Failed to execute 'createImageBitmap' on 'Window': The crop rect height is \
         0.\",\"threw InvalidStateError: Failed to execute 'createImageBitmap' on 'Window': The \
         image source's width is 0.\",\"threw InvalidStateError: Failed to execute \
         'createImageBitmap' on 'Window': The image source is not usable.\",\"threw \
         InvalidStateError: The source image could not be decoded.\",\"threw InvalidStateError: \
         Failed to execute 'createImageBitmap' on 'Window': The options's resizeWidth or \
         resizeHeight is 0.\",\"threw TypeError: Failed to execute 'createImageBitmap' on \
         'Window': Overload resolution failed.\"]",
        "bitmapTainted=\"threw SecurityError: Failed to execute 'getImageData' on \
         'CanvasRenderingContext2D': The canvas has been tainted by cross-origin data.\"",
        "bitmapClosedErrors=[\"threw InvalidStateError: Failed to execute 'createImageBitmap' on \
         'Window': The image source is not usable.\",\"threw InvalidStateError: Failed to \
         execute 'createPattern' on 'CanvasRenderingContext2D': The image source is \
         detached\",\"threw InvalidStateError: Failed to execute 'drawImage' on \
         'CanvasRenderingContext2D': The image source is detached\"]",
        "tags=[\"[object OffscreenCanvas]\",\"[object OffscreenCanvasRenderingContext2D]\",\
         \"function\",0,2,1]",
        "bitmapCtor=\"threw TypeError: Failed to construct 'ImageBitmap': Illegal constructor\"",
    ];
    for line in want {
        assert!(lines.iter().any(|l| l == line), "no {line} in {lines:#?}");
    }
}
