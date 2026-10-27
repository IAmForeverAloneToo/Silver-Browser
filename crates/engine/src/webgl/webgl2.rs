//! WebGL 2's objects (M41 step 8, D40): the buffer bindings it adds and
//! uniform blocks' indexed ones, vertex array objects (WebGL 2's and
//! OES_vertex_array_object's), occlusion queries, fences, and transform
//! feedback's objects (which never record: WebGPU's vertex stage writes
//! no buffer, so beginning one is INVALID_OPERATION, as the note says).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use js::{Answer, CanvasCall};

use super::gl::*;
use super::{Attrib, Context, i, n, u};
use crate::gpu::wire::{Command, Reply, Request};

/// The occlusion slots a query counts draws in; past them a query's
/// further draws are not counted (a query has passed long before).
const OCCLUSION_SLOTS: u32 = 4096;

/// A vertex array object: its attributes and element array buffer.
#[derive(Clone, Debug)]
pub(crate) struct Vao {
    pub attribs: Vec<Attrib>,
    pub element: Option<u32>,
    pub deleted: bool,
    pub bound: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Query {
    pub target: Option<u32>,
    pub deleted: bool,
    /// Its result once the GPU process told it.
    pub result: Arc<Mutex<Option<u64>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct Fence {
    pub signaled: Arc<AtomicBool>,
}

/// The occlusion query set, the buffer it is resolved to, and the one
/// read back; the slots the active query used.
#[derive(Clone, Debug)]
pub(crate) struct Occlusion {
    pub set: u32,
    resolve: u32,
    read: u32,
    pub used: u32,
    pub active: bool,
}

/// A buffer bound at an index: buffer, offset and size (0 for all).
pub(crate) type Indexed = Option<(u32, u64, u64)>;

#[derive(Debug)]
pub(crate) struct State2 {
    pub copy_read: Option<u32>,
    pub copy_write: Option<u32>,
    pub pixel_pack: Option<u32>,
    pub pixel_unpack: Option<u32>,
    pub uniform: Option<u32>,
    pub feedback_buffer: Option<u32>,
    pub uniform_bindings: Vec<Indexed>,
    pub feedback_bindings: Vec<Indexed>,
    pub vaos: HashMap<u32, Vao>,
    pub vao: Option<u32>,
    /// The default vertex array's state while another is bound.
    pub default_vao: Option<(Vec<Attrib>, Option<u32>)>,
    pub queries: HashMap<u32, Query>,
    /// The active queries, by target.
    pub active: HashMap<u32, u32>,
    pub occlusion: Option<Occlusion>,
    pub fences: HashMap<u32, Fence>,
    pub feedbacks: HashMap<u32, (bool, bool)>,
    pub feedback: Option<u32>,
    /// `transformFeedbackVaryings`, kept for its queries.
    pub varyings: HashMap<u32, (Vec<String>, u32)>,
}

impl Default for State2 {
    fn default() -> State2 {
        State2 {
            copy_read: None,
            copy_write: None,
            pixel_pack: None,
            pixel_unpack: None,
            uniform: None,
            feedback_buffer: None,
            uniform_bindings: vec![None; MAX_UNIFORM_BUFFER_BINDINGS_COUNT as usize],
            feedback_bindings: vec![None; 4],
            vaos: HashMap::new(),
            vao: None,
            default_vao: None,
            queries: HashMap::new(),
            active: HashMap::new(),
            occlusion: None,
            fences: HashMap::new(),
            feedbacks: HashMap::new(),
            feedback: None,
            varyings: HashMap::new(),
        }
    }
}

impl State2 {
    /// An occlusion slot for a draw while a query is active.
    pub(crate) fn occlusion_slot(&mut self) -> Option<u32> {
        let o = self.occlusion.as_mut().filter(|o| o.active)?;
        if o.used >= OCCLUSION_SLOTS {
            return None;
        }
        o.used += 1;
        Some(o.used - 1)
    }
}

/// The active uniform blocks of a program: each block, an array's
/// elements one by one: (name, block, element).
pub(crate) fn active_blocks(program: &glsl::Program) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    for (b, block) in program.blocks.iter().enumerate() {
        if block.bindings.len() > 1 {
            for k in 0..block.bindings.len() {
                out.push((format!("{}[{k}]", block.name), b, k));
            }
        } else {
            out.push((block.name.clone(), b, 0));
        }
    }
    out
}

impl Context {
    /// WebGL 2's calls (and OES_vertex_array_object's); none for another.
    pub(super) fn webgl2_op(
        &mut self,
        op: &str,
        call: &CanvasCall,
        size: (u32, u32),
    ) -> Option<Answer> {
        let answer = match op {
            "bindBufferBase" | "bindBufferRange" => {
                self.bind_buffer_indexed(call, op == "bindBufferRange");
                Answer::Nothing
            }
            "getIndexedParameter" => self.indexed_parameter(u(call, 1), u(call, 2)),
            "getBufferSubData" => self.get_buffer_sub_data(u(call, 1), n(call, 2), n(call, 3)),
            "copyBufferSubData" => {
                self.copy_buffer_sub_data(
                    u(call, 1),
                    u(call, 2),
                    n(call, 3),
                    n(call, 4),
                    n(call, 5),
                );
                Answer::Nothing
            }
            // ---- Vertex arrays ----
            "createVertexArray" => {
                let id = self.id();
                self.v2.vaos.insert(
                    id,
                    Vao {
                        attribs: vec![Attrib::default(); 16],
                        element: None,
                        deleted: false,
                        bound: false,
                    },
                );
                Answer::Number(f64::from(id))
            }
            "bindVertexArray" => {
                let id = u(call, 1);
                if id != 0 && !self.v2.vaos.get(&id).is_some_and(|v| !v.deleted) {
                    self.error(INVALID_OPERATION);
                } else {
                    self.bind_vertex_array((id != 0).then_some(id));
                }
                Answer::Nothing
            }
            "deleteVertexArray" => {
                let id = u(call, 1);
                if self.v2.vaos.get(&id).is_some_and(|v| !v.deleted) {
                    if self.v2.vao == Some(id) {
                        self.bind_vertex_array(None);
                    }
                    self.v2.vaos.get_mut(&id).expect("there").deleted = true;
                }
                Answer::Nothing
            }
            "isVertexArray" => Answer::Bool(
                self.v2
                    .vaos
                    .get(&u(call, 1))
                    .is_some_and(|v| !v.deleted && v.bound),
            ),
            // ---- Uniform blocks ----
            "getUniformBlockIndex" => {
                let name = call.strings.first().cloned().unwrap_or_default();
                match self.linked_of(u(call, 1)) {
                    Some(linked) => {
                        let index = active_blocks(&linked.program)
                            .iter()
                            .position(|(n, _, _)| *n == name)
                            .map_or(INVALID_INDEX, |k| k as u32);
                        Answer::Number(f64::from(index))
                    }
                    None => Answer::Number(f64::from(INVALID_INDEX)),
                }
            }
            "uniformBlockBinding" => {
                let (program, index, binding) = (u(call, 1), u(call, 2) as usize, u(call, 3));
                let count = self
                    .linked_of(program)
                    .map(|l| active_blocks(&l.program).len());
                match count {
                    None => {}
                    Some(c) if index >= c || binding >= MAX_UNIFORM_BUFFER_BINDINGS_COUNT => {
                        self.error(INVALID_VALUE)
                    }
                    Some(_) => {
                        let linked = self
                            .programs
                            .get_mut(&program)
                            .and_then(|p| p.linked.as_mut())
                            .expect("linked");
                        linked.block_bindings[index] = binding;
                    }
                }
                Answer::Nothing
            }
            "getActiveUniformBlockParameter" => {
                self.block_parameter(u(call, 1), u(call, 2) as usize, u(call, 3))
            }
            "getActiveUniformBlockName" => match self.linked_of(u(call, 1)) {
                Some(linked) => match active_blocks(&linked.program).get(u(call, 2) as usize) {
                    Some((name, _, _)) => Answer::MaybeString(Some(name.clone())),
                    None => {
                        self.error(INVALID_VALUE);
                        Answer::MaybeString(None)
                    }
                },
                None => Answer::MaybeString(None),
            },
            "getUniformIndices" => match self.linked_of(u(call, 1)) {
                Some(linked) => {
                    let active = super::active_uniforms(&linked.program);
                    Answer::Numbers(
                        call.strings
                            .iter()
                            .map(|name| {
                                let plain = |n: &str| n.strip_suffix("[0]").unwrap_or(n).to_owned();
                                active
                                    .iter()
                                    .position(|(a, _, _)| a == name || plain(a) == *name)
                                    .map_or(f64::from(INVALID_INDEX), |k| k as f64)
                            })
                            .collect(),
                    )
                }
                None => Answer::Nothing,
            },
            "getActiveUniforms" => self.active_uniforms_parameter(call),
            // ---- Queries ----
            "createQuery" => {
                let id = self.id();
                self.v2.queries.insert(
                    id,
                    Query {
                        target: None,
                        deleted: false,
                        result: Arc::new(Mutex::new(None)),
                    },
                );
                Answer::Number(f64::from(id))
            }
            "deleteQuery" => {
                let id = u(call, 1);
                if let Some(q) = self.v2.queries.get_mut(&id) {
                    q.deleted = true;
                    if let Some(target) = self
                        .v2
                        .active
                        .iter()
                        .find(|(_, a)| **a == id)
                        .map(|(&t, _)| t)
                    {
                        self.end_query(target);
                    }
                }
                Answer::Nothing
            }
            "isQuery" => Answer::Bool(
                self.v2
                    .queries
                    .get(&u(call, 1))
                    .is_some_and(|q| !q.deleted && q.target.is_some()),
            ),
            "beginQuery" => {
                self.begin_query(u(call, 1), u(call, 2));
                Answer::Nothing
            }
            "endQuery" => {
                let target = u(call, 1);
                if !matches!(
                    target,
                    ANY_SAMPLES_PASSED
                        | ANY_SAMPLES_PASSED_CONSERVATIVE
                        | TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN
                ) {
                    self.error(INVALID_ENUM);
                } else if !self.v2.active.contains_key(&target) {
                    self.error(INVALID_OPERATION);
                } else {
                    self.end_query(target);
                }
                Answer::Nothing
            }
            "getQuery" => {
                let (target, name) = (u(call, 1), u(call, 2));
                if !matches!(
                    target,
                    ANY_SAMPLES_PASSED
                        | ANY_SAMPLES_PASSED_CONSERVATIVE
                        | TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN
                ) || name != CURRENT_QUERY
                {
                    self.error(INVALID_ENUM);
                    Answer::Nothing
                } else {
                    // Either occlusion target names the one active.
                    let found = self.v2.active.get(&target).copied().or_else(|| {
                        if target == TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN {
                            None
                        } else {
                            self.v2
                                .active
                                .get(&ANY_SAMPLES_PASSED)
                                .or_else(|| self.v2.active.get(&ANY_SAMPLES_PASSED_CONSERVATIVE))
                                .copied()
                                .filter(|&q| self.v2.queries[&q].target == Some(target))
                        }
                    });
                    Answer::Number(found.map_or(0.0, f64::from))
                }
            }
            "getQueryParameter" => {
                let (id, name) = (u(call, 1), u(call, 2));
                let Some(q) = self.v2.queries.get(&id).filter(|q| !q.deleted) else {
                    self.error(INVALID_OPERATION);
                    return Some(Answer::Nothing);
                };
                if q.target.is_none() || self.v2.active.values().any(|&a| a == id) {
                    self.error(INVALID_OPERATION);
                    return Some(Answer::Nothing);
                }
                let result = *q.result.lock().expect("result");
                match name {
                    QUERY_RESULT_AVAILABLE => Answer::Bool(result.is_some()),
                    QUERY_RESULT => Answer::Number(result.unwrap_or(0) as f64),
                    _ => {
                        self.error(INVALID_ENUM);
                        Answer::Nothing
                    }
                }
            }
            // ---- Fences ----
            "fenceSync" => {
                if u(call, 1) != SYNC_GPU_COMMANDS_COMPLETE {
                    self.error(INVALID_ENUM);
                    return Some(Answer::Number(0.0));
                }
                if u(call, 2) != 0 {
                    self.error(INVALID_VALUE);
                    return Some(Answer::Number(0.0));
                }
                let id = self.id();
                let signaled = Arc::new(AtomicBool::new(false));
                self.gpu_flush();
                let told = Arc::clone(&signaled);
                let device = self.device;
                self.client.call(
                    |call| Request::WorkDone { call, device },
                    move |_, _| told.store(true, Ordering::SeqCst),
                );
                self.client.flush();
                self.v2.fences.insert(id, Fence { signaled });
                Answer::Number(f64::from(id))
            }
            "deleteSync" => {
                self.v2.fences.remove(&u(call, 1));
                Answer::Nothing
            }
            "isSync" => Answer::Bool(self.v2.fences.contains_key(&u(call, 1))),
            "syncSignaled" => Answer::Bool(
                self.v2
                    .fences
                    .get(&u(call, 1))
                    .is_some_and(|f| f.signaled.load(Ordering::SeqCst)),
            ),
            "getSyncParameter" => {
                let (id, name) = (u(call, 1), u(call, 2));
                let Some(fence) = self.v2.fences.get(&id) else {
                    self.error(INVALID_VALUE);
                    return Some(Answer::Nothing);
                };
                let signaled = fence.signaled.load(Ordering::SeqCst);
                match name {
                    OBJECT_TYPE => Answer::Number(f64::from(SYNC_FENCE)),
                    SYNC_STATUS => {
                        Answer::Number(f64::from(if signaled { SIGNALED } else { UNSIGNALED }))
                    }
                    SYNC_CONDITION => Answer::Number(f64::from(SYNC_GPU_COMMANDS_COMPLETE)),
                    SYNC_FLAGS => Answer::Number(0.0),
                    _ => {
                        self.error(INVALID_ENUM);
                        Answer::Nothing
                    }
                }
            }
            "clientWaitSync" => {
                let (id, flags, timeout) = (u(call, 1), u(call, 2), n(call, 3));
                if !self.v2.fences.contains_key(&id) {
                    self.error(INVALID_VALUE);
                    return Some(Answer::Number(f64::from(WAIT_FAILED)));
                }
                if flags & !SYNC_FLUSH_COMMANDS_BIT != 0 {
                    self.error(INVALID_VALUE);
                    return Some(Answer::Number(f64::from(WAIT_FAILED)));
                }
                // MAX_CLIENT_WAIT_TIMEOUT_WEBGL is 0: WebGL never blocks.
                if timeout > 0.0 {
                    self.error(INVALID_OPERATION);
                    return Some(Answer::Number(f64::from(WAIT_FAILED)));
                }
                let signaled = self.v2.fences[&id].signaled.load(Ordering::SeqCst);
                Answer::Number(f64::from(if signaled {
                    ALREADY_SIGNALED
                } else {
                    TIMEOUT_EXPIRED
                }))
            }
            "waitSync" => {
                let (id, flags, timeout) = (u(call, 1), u(call, 2), n(call, 3));
                if !self.v2.fences.contains_key(&id) || flags != 0 || timeout != -1.0 {
                    self.error(INVALID_VALUE);
                }
                Answer::Nothing
            }
            // ---- Transform feedback: its objects, never recording ----
            "createTransformFeedback" => {
                let id = self.id();
                self.v2.feedbacks.insert(id, (false, false));
                Answer::Number(f64::from(id))
            }
            "bindTransformFeedback" => {
                let (target, id) = (u(call, 1), u(call, 2));
                if target != TRANSFORM_FEEDBACK {
                    self.error(INVALID_ENUM);
                } else if id == 0 {
                    self.v2.feedback = None;
                } else {
                    match self.v2.feedbacks.get_mut(&id) {
                        Some(f) if !f.0 => {
                            f.1 = true;
                            self.v2.feedback = Some(id);
                        }
                        _ => self.error(INVALID_OPERATION),
                    }
                }
                Answer::Nothing
            }
            "deleteTransformFeedback" => {
                let id = u(call, 1);
                if let Some(f) = self.v2.feedbacks.get_mut(&id) {
                    f.0 = true;
                    if self.v2.feedback == Some(id) {
                        self.v2.feedback = None;
                    }
                }
                Answer::Nothing
            }
            "isTransformFeedback" => Answer::Bool(
                self.v2
                    .feedbacks
                    .get(&u(call, 1))
                    .is_some_and(|f| !f.0 && f.1),
            ),
            "beginTransformFeedback"
            | "endTransformFeedback"
            | "pauseTransformFeedback"
            | "resumeTransformFeedback" => {
                self.error(INVALID_OPERATION);
                Answer::Nothing
            }
            "transformFeedbackVaryings" => {
                let (program, mode) = (u(call, 1), u(call, 2));
                if !matches!(mode, INTERLEAVED_ATTRIBS | SEPARATE_ATTRIBS) {
                    self.error(INVALID_ENUM);
                } else if !self.programs.contains_key(&program) {
                    self.error(INVALID_VALUE);
                } else {
                    self.v2
                        .varyings
                        .insert(program, (call.strings.clone(), mode));
                }
                Answer::Nothing
            }
            "getTransformFeedbackVarying" => {
                self.error(INVALID_VALUE);
                Answer::Nothing
            }
            "drawRangeElements" => {
                // [context, mode, start, end, count, type, offset].
                if i(call, 3) < i(call, 2) {
                    self.error(INVALID_VALUE);
                } else {
                    self.draw_elements(u(call, 1), i(call, 4), u(call, 5), n(call, 6), 1, size);
                }
                Answer::Nothing
            }
            _ => return None,
        };
        Some(answer)
    }

    /// A program's link when it has one (else GL's error).
    fn linked_of(&mut self, program: u32) -> Option<super::Linked> {
        let Some(p) = self.programs.get(&program) else {
            self.error(INVALID_VALUE);
            return None;
        };
        match p.linked.clone().filter(|_| p.link_status) {
            Some(l) => Some(l),
            None => {
                self.error(INVALID_OPERATION);
                None
            }
        }
    }

    /// The vertex array bound: the one before saved, the new one's state
    /// made the context's.
    fn bind_vertex_array(&mut self, id: Option<u32>) {
        if self.v2.vao == id {
            return;
        }
        let attribs = std::mem::take(&mut self.attribs);
        let element = self.element_buffer.take();
        match self.v2.vao {
            Some(old) => {
                if let Some(v) = self.v2.vaos.get_mut(&old) {
                    v.attribs = attribs;
                    v.element = element;
                }
            }
            None => self.v2.default_vao = Some((attribs, element)),
        }
        match id {
            Some(new) => {
                let v = self.v2.vaos.get_mut(&new).expect("checked");
                v.bound = true;
                self.attribs = std::mem::take(&mut v.attribs);
                self.element_buffer = v.element.take();
            }
            None => {
                let (attribs, element) = self.v2.default_vao.take().expect("saved");
                self.attribs = attribs;
                self.element_buffer = element;
            }
        }
        self.v2.vao = id;
    }

    fn bind_buffer_indexed(&mut self, call: &CanvasCall, range: bool) {
        let (target, index, id) = (u(call, 1), u(call, 2) as usize, u(call, 3));
        let (offset, size) = if range {
            (n(call, 4), n(call, 5))
        } else {
            (0.0, 0.0)
        };
        let count = match target {
            UNIFORM_BUFFER => MAX_UNIFORM_BUFFER_BINDINGS_COUNT as usize,
            TRANSFORM_FEEDBACK_BUFFER => 4,
            _ => {
                self.error(INVALID_ENUM);
                return;
            }
        };
        if index >= count {
            self.error(INVALID_VALUE);
            return;
        }
        if id != 0 {
            match self.buffers.get_mut(&id) {
                Some(b) if !b.deleted => {
                    if b.target == Some(ELEMENT_ARRAY_BUFFER) {
                        self.error(INVALID_OPERATION);
                        return;
                    }
                    b.target.get_or_insert(target);
                }
                _ => {
                    self.error(INVALID_OPERATION);
                    return;
                }
            }
        }
        if range && id != 0 {
            let aligned = if target == UNIFORM_BUFFER { 256.0 } else { 4.0 };
            if offset < 0.0 || size <= 0.0 || offset % aligned != 0.0 {
                self.error(INVALID_VALUE);
                return;
            }
        }
        let value = (id != 0).then_some((id, offset as u64, size as u64));
        if target == UNIFORM_BUFFER {
            self.v2.uniform_bindings[index] = value;
            self.v2.uniform = (id != 0).then_some(id);
        } else {
            self.v2.feedback_bindings[index] = value;
            self.v2.feedback_buffer = (id != 0).then_some(id);
        }
    }

    fn indexed_parameter(&mut self, target: u32, index: u32) -> Answer {
        let (list, part) = match target {
            UNIFORM_BUFFER_BINDING | UNIFORM_BUFFER_START | UNIFORM_BUFFER_SIZE => {
                (&self.v2.uniform_bindings, target - UNIFORM_BUFFER_BINDING)
            }
            TRANSFORM_FEEDBACK_BUFFER_BINDING => (&self.v2.feedback_bindings, 0),
            TRANSFORM_FEEDBACK_BUFFER_START => (&self.v2.feedback_bindings, 1),
            TRANSFORM_FEEDBACK_BUFFER_SIZE => (&self.v2.feedback_bindings, 2),
            _ => {
                self.error(INVALID_ENUM);
                return Answer::Nothing;
            }
        };
        let Some(bound) = list.get(index as usize).copied() else {
            self.error(INVALID_VALUE);
            return Answer::Nothing;
        };
        let (id, offset, size) = bound.unwrap_or((0, 0, 0));
        Answer::Number(match part {
            0 => f64::from(id),
            1 => offset as f64,
            _ => size as f64,
        })
    }

    fn get_buffer_sub_data(&mut self, target: u32, offset: f64, length: f64) -> Answer {
        let bound = match self.bound_buffer(target) {
            Ok(b) => b,
            Err(e) => {
                self.error(e);
                return Answer::Nothing;
            }
        };
        let Some(id) = bound else {
            self.error(INVALID_OPERATION);
            return Answer::Nothing;
        };
        let data = &self.buffers[&id].data;
        if offset < 0.0 || length < 0.0 || offset + length > data.len() as f64 {
            self.error(INVALID_VALUE);
            return Answer::Nothing;
        }
        Answer::Bytes(data[offset as usize..(offset + length) as usize].to_vec())
    }

    fn copy_buffer_sub_data(&mut self, read: u32, write: u32, from: f64, to: f64, size: f64) {
        let (Ok(source), Ok(dest)) = (self.bound_buffer(read), self.bound_buffer(write)) else {
            self.error(INVALID_ENUM);
            return;
        };
        let (Some(source), Some(dest)) = (source, dest) else {
            self.error(INVALID_OPERATION);
            return;
        };
        let element = |id: u32| self.buffers[&id].target == Some(ELEMENT_ARRAY_BUFFER);
        if element(source) != element(dest) {
            self.error(INVALID_OPERATION);
            return;
        }
        let (source_len, dest_len) = (
            self.buffers[&source].data.len() as f64,
            self.buffers[&dest].data.len() as f64,
        );
        if from < 0.0 || to < 0.0 || size < 0.0 || from + size > source_len || to + size > dest_len
        {
            self.error(INVALID_VALUE);
            return;
        }
        if source == dest && (from - to).abs() < size {
            self.error(INVALID_VALUE);
            return;
        }
        let bytes = self.buffers[&source].data[from as usize..(from + size) as usize].to_vec();
        self.buffer_sub_data(dest, to as usize, &bytes);
    }

    fn block_parameter(&mut self, program: u32, index: usize, name: u32) -> Answer {
        let Some(linked) = self.linked_of(program) else {
            return Answer::Nothing;
        };
        let blocks = active_blocks(&linked.program);
        let Some(&(_, b, _)) = blocks.get(index) else {
            self.error(INVALID_VALUE);
            return Answer::Nothing;
        };
        let block = &linked.program.blocks[b];
        let members: Vec<f64> = super::active_uniforms(&linked.program)
            .iter()
            .enumerate()
            .filter(|(_, (name, _, _))| {
                linked
                    .program
                    .uniforms
                    .iter()
                    .any(|u| u.block == Some(b) && u.name == *name)
            })
            .map(|(k, _)| k as f64)
            .collect();
        match name {
            UNIFORM_BLOCK_BINDING => Answer::Number(f64::from(linked.block_bindings[index])),
            UNIFORM_BLOCK_DATA_SIZE => Answer::Number(f64::from(block.size)),
            UNIFORM_BLOCK_ACTIVE_UNIFORMS => Answer::Number(members.len() as f64),
            UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES => Answer::Numbers(members),
            UNIFORM_BLOCK_REFERENCED_BY_VERTEX_SHADER => Answer::Bool(block.vertex),
            UNIFORM_BLOCK_REFERENCED_BY_FRAGMENT_SHADER => Answer::Bool(block.fragment),
            _ => {
                self.error(INVALID_ENUM);
                Answer::Nothing
            }
        }
    }

    /// `getActiveUniforms` [context, program, pname, indices...].
    fn active_uniforms_parameter(&mut self, call: &CanvasCall) -> Answer {
        let (program, name) = (u(call, 1), u(call, 2));
        let Some(linked) = self.linked_of(program) else {
            return Answer::Nothing;
        };
        let active = super::active_uniforms(&linked.program);
        let indices: Vec<usize> = call.numbers.iter().skip(3).map(|&v| v as usize).collect();
        if indices.iter().any(|&k| k >= active.len()) {
            self.error(INVALID_VALUE);
            return Answer::Nothing;
        }
        // An active uniform's own record, a sampler none.
        let record = |k: usize| {
            let name = &active[k].0;
            linked.program.uniforms.iter().find(|u| u.name == *name)
        };
        let blocks = active_blocks(&linked.program);
        let mut out = Vec::new();
        for k in indices {
            let r = record(k);
            let v = match name {
                UNIFORM_TYPE => f64::from(active[k].2),
                UNIFORM_SIZE => f64::from(active[k].1),
                UNIFORM_BLOCK_INDEX => r
                    .and_then(|u| u.block)
                    .and_then(|b| blocks.iter().position(|(_, bb, _)| *bb == b))
                    .map_or(-1.0, |p| p as f64),
                UNIFORM_OFFSET => r
                    .filter(|u| u.block.is_some())
                    .map_or(-1.0, |u| f64::from(u.offset)),
                UNIFORM_ARRAY_STRIDE => r
                    .filter(|u| u.block.is_some())
                    .map_or(-1.0, |u| f64::from(u.array_stride)),
                UNIFORM_MATRIX_STRIDE => r
                    .filter(|u| u.block.is_some())
                    .map_or(-1.0, |u| f64::from(u.matrix_stride)),
                UNIFORM_IS_ROW_MAJOR => f64::from(u8::from(r.is_some_and(|u| u.row_major))),
                _ => {
                    self.error(INVALID_ENUM);
                    return Answer::Nothing;
                }
            };
            out.push(v);
        }
        Answer::Numbers(out)
    }

    fn begin_query(&mut self, target: u32, id: u32) {
        if !matches!(
            target,
            ANY_SAMPLES_PASSED
                | ANY_SAMPLES_PASSED_CONSERVATIVE
                | TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN
        ) {
            self.error(INVALID_ENUM);
            return;
        }
        let occlusion = target != TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN;
        // One occlusion query at a time, of either target.
        let busy = if occlusion {
            self.v2.active.contains_key(&ANY_SAMPLES_PASSED)
                || self
                    .v2
                    .active
                    .contains_key(&ANY_SAMPLES_PASSED_CONSERVATIVE)
        } else {
            self.v2.active.contains_key(&target)
        };
        let Some(q) = self.v2.queries.get_mut(&id).filter(|q| !q.deleted) else {
            self.error(INVALID_OPERATION);
            return;
        };
        if busy
            || q.target.is_some_and(|t| t != target)
            || self.v2.active.values().any(|&a| a == id)
        {
            self.error(INVALID_OPERATION);
            return;
        }
        q.target = Some(target);
        *q.result.lock().expect("result") = None;
        self.v2.active.insert(target, id);
        if !occlusion {
            return;
        }
        if self.v2.occlusion.is_none() {
            let (set, resolve, read) = (
                self.gpu.id(&self.client),
                self.gpu.id(&self.client),
                self.gpu.id(&self.client),
            );
            let bytes = u64::from(OCCLUSION_SLOTS) * 8;
            self.client.queue([
                Request::CreateQuerySet {
                    device: self.device,
                    id: set,
                    kind: 0,
                    count: OCCLUSION_SLOTS,
                },
                // QUERY_RESOLVE | COPY_SRC; MAP_READ | COPY_DST.
                Request::CreateBuffer {
                    device: self.device,
                    id: resolve,
                    size: bytes,
                    usage: 0x204,
                    mapped: false,
                },
                Request::CreateBuffer {
                    device: self.device,
                    id: read,
                    size: bytes,
                    usage: 9,
                    mapped: false,
                },
            ]);
            self.v2.occlusion = Some(Occlusion {
                set,
                resolve,
                read,
                used: 0,
                active: false,
            });
        }
        // The next pass counts into the set.
        self.end_pass();
        let o = self.v2.occlusion.as_mut().expect("made");
        o.used = 0;
        o.active = true;
    }

    /// The query of `target` ended: its slots resolved and read back, its
    /// result told when they come (none drawn: false at once).
    pub(crate) fn end_query(&mut self, target: u32) {
        let Some(id) = self.v2.active.remove(&target) else {
            return;
        };
        let result = Arc::clone(&self.v2.queries[&id].result);
        let occlusion = target != TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN;
        let used = match self.v2.occlusion.as_mut() {
            Some(o) if occlusion => {
                o.active = false;
                o.used
            }
            _ => 0,
        };
        if used == 0 {
            *result.lock().expect("result") = Some(0);
            return;
        }
        let o = self.v2.occlusion.clone().expect("made");
        self.end_pass();
        self.gpu.pending.push(Command::ResolveQuerySet {
            set: o.set,
            first: 0,
            count: used,
            destination: o.resolve,
            offset: 0,
        });
        self.gpu.pending.push(Command::CopyBufferToBuffer {
            source: o.resolve,
            source_offset: 0,
            destination: o.read,
            destination_offset: 0,
            size: Some(u64::from(used) * 8),
        });
        self.gpu_flush();
        let read = o.read;
        self.client.call(
            |call| Request::MapBuffer {
                call,
                buffer: read,
                mode: 1,
                offset: 0,
                size: u64::from(used) * 8,
            },
            move |reply, bytes| {
                let any = matches!(reply, Reply::Mapped { error: None, .. })
                    && bytes
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .any(|c| u64::from_le_bytes(*c) != 0);
                *result.lock().expect("result") = Some(u64::from(any));
            },
        );
        self.client.queue([Request::Unmap { buffer: read }]);
        self.client.flush();
    }
}
