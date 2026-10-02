//! Arrays in linear memory (see `layout::ARR_*`).
//!
//! Arrays used only by compiled code live in the module's memory so that
//! their elements are read and written by wasm instructions. Arrays the
//! interpreter must see (used by instructions it runs: Varptr, Array(),
//! Input, dialogs...) stay in the interpreter; the compiler decides, per
//! array, consistently for the whole program.
//!
//! Storage: natively the blocks are in the module's own memory, after the
//! procedure frames, and the memory grows on demand ([`HeapKind::Linear`]);
//! on the web the module shares the runtime's memory and each block is a
//! separate allocation of the runtime ([`HeapKind::Owned`]).

use super::*;
use crate::compiled::layout::{ARR_COUNT, ARR_DATA, ARR_DIMS, ARR_NDIMS, ARR_TYPE, elem_size};

/// Where array blocks are allocated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeapKind {
    /// In the module's memory (base 0), which the host can grow.
    Linear,
    /// Allocations of the runtime, in a memory shared with the module
    /// (wasm32 only).
    Owned,
}

pub(super) struct Block {
    pub addr: u32,
    pub size: u32,
    pub ty: u8,
    pub count: u32,
    /// Procedure depth of the variable (0: global): freed when that frame
    /// is popped.
    pub owner: usize,
}

pub(super) struct Heap {
    pub kind: HeapKind,
    /// Linear: end of the used part (relative to the base).
    top: u32,
    /// Linear: free ranges (relative start, size), sorted.
    free: Vec<(u32, u32)>,
    pub blocks: Vec<Block>,
    /// Owned: the allocations (absolute address, pointer).
    owned: Vec<(u32, *mut [u8])>,
    /// Linear: bytes the memory must grow by for the last allocation.
    pub grow_bytes: u32,
}

impl Drop for Heap {
    fn drop(&mut self) {
        for (_, p) in self.owned.drain(..) {
            // SAFETY: allocated by `Box::into_raw` in `alloc`.
            drop(unsafe { Box::from_raw(p) });
        }
    }
}

impl Heap {
    pub fn new(kind: HeapKind, start: u32) -> Heap {
        Heap {
            kind,
            top: start.next_multiple_of(16),
            free: Vec::new(),
            blocks: Vec::new(),
            owned: Vec::new(),
            grow_bytes: 0,
        }
    }

    /// Allocates `size` zeroed bytes; `None` if the memory must grow first
    /// (Linear, `grow_bytes` set).
    fn alloc(&mut self, mem: &mut [u8], base: u32, size: u32) -> Option<u32> {
        let size = size.next_multiple_of(16);
        match self.kind {
            HeapKind::Linear => {
                let rel = if let Some(i) = self.free.iter().position(|f| f.1 >= size) {
                    let (start, len) = self.free[i];
                    if len == size {
                        self.free.remove(i);
                    } else {
                        self.free[i] = (start + size, len - size);
                    }
                    start
                } else {
                    let end = self.top as u64 + size as u64;
                    if end > mem.len() as u64 {
                        self.grow_bytes = (end - mem.len() as u64).min(u32::MAX as u64) as u32;
                        return None;
                    }
                    let start = self.top;
                    self.top += size;
                    start
                };
                mem[rel as usize..(rel + size) as usize].fill(0);
                Some(base + rel)
            }
            HeapKind::Owned => {
                #[cfg(target_arch = "wasm32")]
                {
                    let p: *mut [u8] = Box::into_raw(vec![0u8; size as usize].into_boxed_slice());
                    let addr = p as *mut u8 as usize as u32;
                    self.owned.push((addr, p));
                    Some(addr)
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let _ = (mem, base);
                    panic!("owned array blocks need a 32 bit address space")
                }
            }
        }
    }

    fn release(&mut self, base: u32, addr: u32, size: u32) {
        match self.kind {
            HeapKind::Linear => {
                let (start, size) = (addr - base, size.next_multiple_of(16));
                let i = self.free.partition_point(|f| f.0 < start);
                self.free.insert(i, (start, size));
                // Merge with the neighbours.
                if i + 1 < self.free.len() && self.free[i].0 + self.free[i].1 == self.free[i + 1].0 {
                    self.free[i].1 += self.free[i + 1].1;
                    self.free.remove(i + 1);
                }
                if i > 0 && self.free[i - 1].0 + self.free[i - 1].1 == self.free[i].0 {
                    self.free[i - 1].1 += self.free[i].1;
                    self.free.remove(i);
                }
                // Give back the end of the heap.
                if let Some(&(s, l)) = self.free.last()
                    && s + l == self.top
                {
                    self.top = s;
                    self.free.pop();
                }
            }
            HeapKind::Owned => {
                if let Some(i) = self.owned.iter().position(|o| o.0 == addr) {
                    let (_, p) = self.owned.swap_remove(i);
                    // SAFETY: allocated by `Box::into_raw` in `alloc`.
                    drop(unsafe { Box::from_raw(p) });
                }
            }
        }
    }

    /// The bytes of a block.
    pub fn bytes<'m>(&self, mem: &'m mut [u8], base: u32, addr: u32, len: u32) -> &'m mut [u8] {
        match self.kind {
            HeapKind::Linear => &mut mem[(addr - base) as usize..(addr - base + len) as usize],
            HeapKind::Owned => {
                let _ = mem;
                // SAFETY: `addr` is a live allocation of `len` bytes or
                // more made by `alloc` (blocks are only looked up through
                // `blocks`); the module only accesses it during calls.
                unsafe { std::slice::from_raw_parts_mut(addr as usize as *mut u8, len as usize) }
            }
        }
    }
}

impl Runtime {
    /// Dimensions array `slot` (type `ty`) with the `n` maximum indices in
    /// the `IDX` area, already checked by the module (`Interp::dim`).
    /// `resident`: the array lives in the interpreter. Returns a status,
    /// or `ST_GROW` when the memory must grow by `grow_bytes()` first.
    #[allow(clippy::too_many_arguments)]
    pub fn dim(
        &mut self,
        env: &mut dyn Env,
        mem: &mut [u8],
        pos: i32,
        slot: i32,
        ty: i32,
        n: i32,
        resident: i32,
    ) -> i32 {
        let (it, hw) = env.parts();
        let pos = pos as usize;
        it.inst_pos = pos;
        let slot = slot as u16;
        let n = (n.clamp(0, 8)) as usize;
        let dims: Vec<u16> = (0..n).map(|k| ld_i32(mem, layout::IDX + k as u32 * 4) as u16).collect();
        let r = if resident != 0 {
            let var = it.var_slot(slot);
            if matches!(var, Var::Array(_)) {
                Err(Exc::Error(errors::ARRAY_ALREADY_DIMENSIONED))
            } else {
                *var = Var::Array(Box::new(crate::interp::value::Array::new(ty as u8, dims)));
                Ok(ST_CONTINUE)
            }
        } else {
            let depth = it.frame_stack.len();
            let slot_addr = self.scalar_addr(slot, depth);
            if ld_i32(mem, slot_addr) != 0 {
                Err(Exc::Error(errors::ARRAY_ALREADY_DIMENSIONED))
            } else {
                let count: u64 = dims.iter().map(|&d| d as u64 + 1).product();
                let size = ARR_DATA as u64 + count * elem_size(ty as u8) as u64;
                if size > u32::MAX as u64 / 2 {
                    Err(Exc::Error(errors::OUT_OF_MEMORY))
                } else {
                    match self.heap.alloc(mem, self.base, size as u32) {
                        None => return ST_GROW,
                        Some(addr) => {
                            let b = self.heap.bytes(mem, self.base, addr, ARR_DATA);
                            let put = |b: &mut [u8], off: u32, v: u32| {
                                b[off as usize..off as usize + 4].copy_from_slice(&v.to_le_bytes())
                            };
                            put(b, ARR_NDIMS, n as u32);
                            put(b, ARR_TYPE, ty as u32);
                            put(b, ARR_COUNT, count as u32);
                            for (k, &d) in dims.iter().enumerate() {
                                put(b, ARR_DIMS + k as u32 * 4, d as u32);
                            }
                            let owner = if slot & GLOBAL != 0 || depth == 0 { 0 } else { depth };
                            self.heap.blocks.push(Block {
                                addr,
                                size: size as u32,
                                ty: ty as u8,
                                count: count as u32,
                                owner,
                            });
                            st_i32(mem, slot_addr, addr as i32);
                            Ok(ST_CONTINUE)
                        }
                    }
                }
            }
        };
        self.result(it, hw, mem, r, pos)
    }

    /// Bytes the memory must grow by after `dim` returned `ST_GROW`.
    pub fn grow_bytes(&self) -> u32 {
        self.heap.grow_bytes
    }

    /// The memory could not grow: Out of memory for the Dim at `pos`.
    pub fn grow_failed(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32) -> i32 {
        let (it, hw) = env.parts();
        self.result(it, hw, mem, Err(Exc::Error(errors::OUT_OF_MEMORY)), pos as usize)
    }

    /// Frees the arrays of the procedure frames deeper than `depth`.
    pub(super) fn free_arrays_above(&mut self, depth: usize) {
        let base = self.base;
        let mut i = 0;
        while i < self.heap.blocks.len() {
            if self.heap.blocks[i].owner > depth {
                let b = self.heap.blocks.swap_remove(i);
                self.heap.release(base, b.addr, b.size);
            } else {
                i += 1;
            }
        }
    }

    /// Marks the string handles of the string arrays (GC).
    pub(super) fn mark_arrays(&self, mem: &mut [u8], mark: &mut [bool]) {
        for b in self.heap.blocks.iter().filter(|b| b.ty == 2) {
            let data = self.heap.bytes(mem, self.base, b.addr + ARR_DATA, b.count * 4);
            for c in data.as_chunks::<4>().0 {
                let h = i32::from_le_bytes(*c);
                if h > 0 && (h as usize) < mark.len() {
                    mark[h as usize] = true;
                }
            }
        }
    }

    /// Type and element count of the array block at `addr`.
    pub(super) fn array_block(&self, addr: u32) -> Option<(u8, u32)> {
        self.block(addr)
    }

    /// The block of the array at `addr`.
    fn block(&self, addr: u32) -> Option<(u8, u32)> {
        self.heap.blocks.iter().find(|b| b.addr == addr).map(|b| (b.ty, b.count))
    }

    /// `Sort a(0)` on an array in memory (`exec_core` SORT).
    pub fn sort_array(&mut self, mem: &mut [u8], addr: i32) {
        let Some((ty, count)) = self.block(addr as u32) else { return };
        let es = elem_size(ty);
        let data = self.heap.bytes(mem, self.base, addr as u32 + ARR_DATA, count * es);
        match ty {
            0 => {
                let mut v: Vec<i32> = data.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes(*c)).collect();
                v.sort();
                for (c, x) in data.as_chunks_mut::<4>().0.iter_mut().zip(v) {
                    c.copy_from_slice(&x.to_le_bytes());
                }
            }
            1 => {
                let mut v: Vec<f64> = data.as_chunks::<8>().0.iter().map(|c| f64::from_le_bytes(*c)).collect();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                for (c, x) in data.as_chunks_mut::<8>().0.iter_mut().zip(v) {
                    c.copy_from_slice(&x.to_le_bytes());
                }
            }
            _ => {
                let mut v: Vec<(AStr, i32)> = data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| {
                        let h = i32::from_le_bytes(*c);
                        (self.str_of(h), h)
                    })
                    .collect();
                // Stable, like `Vec<AStr>::sort`.
                v.sort_by(|a, b| a.0[..].cmp(&b.0[..]));
                let data = self.heap.bytes(mem, self.base, addr as u32 + ARR_DATA, count * es);
                for (c, (_, h)) in data.as_chunks_mut::<4>().0.iter_mut().zip(v) {
                    c.copy_from_slice(&h.to_le_bytes());
                }
            }
        }
    }

    /// `Match(a(0), v)` on an array in memory (`array_match` in
    /// `interp/expr.rs`): the value is in `RET` with its type in `RET_TAG`
    /// (already converted to the array type).
    pub fn match_array(&mut self, mem: &mut [u8], addr: i32) -> i32 {
        use std::cmp::Ordering;
        let Some((ty, n)) = self.block(addr as u32) else { return 0 };
        let n = n as usize;
        let es = elem_size(ty);
        let (vi, vf, vs) = (ld_i32(mem, layout::RET), ld_f64(mem, layout::RET), self.str_of(ld_i32(mem, layout::RET)));
        let data = self.heap.bytes(mem, self.base, addr as u32 + ARR_DATA, n as u32 * es).to_vec();
        let cmp = |i: usize| -> Ordering {
            let c = &data[i * es as usize..];
            match ty {
                0 => i32::from_le_bytes(c[..4].try_into().unwrap()).cmp(&vi),
                1 => f64::from_le_bytes(c[..8].try_into().unwrap()).partial_cmp(&vf).unwrap_or(Ordering::Equal),
                _ => self.str_of(i32::from_le_bytes(c[..4].try_into().unwrap()))[..].cmp(&vs[..]),
            }
        };
        match_search(n, cmp)
    }

    /// `Match` on an array kept in the interpreter (value in `RET`).
    pub fn match_resident(&mut self, env: &mut dyn Env, mem: &mut [u8], pos: i32, slot: i32) -> i32 {
        use crate::interp::value::ArrayData;
        use std::cmp::Ordering;
        let (it, _) = env.parts();
        it.inst_pos = pos as usize;
        let (vi, vf, vs) = (ld_i32(mem, layout::RET), ld_f64(mem, layout::RET), self.str_of(ld_i32(mem, layout::RET)));
        match it.var_slot(slot as u16) {
            Var::Array(a) => match &a.data {
                ArrayData::Int(v) => match_search(v.len(), |i| v[i].cmp(&vi)),
                ArrayData::Float(v) => match_search(v.len(), |i| v[i].partial_cmp(&vf).unwrap_or(Ordering::Equal)),
                ArrayData::Str(v) => match_search(v.len(), |i| v[i][..].cmp(&vs[..])),
            },
            _ => {
                self.set_pending(mem, Exc::Error(errors::NON_DIMENSIONED_ARRAY));
                0
            }
        }
    }
}

/// Binary then linear search of `Match` (`array_match` in `interp/expr.rs`).
fn match_search(n: usize, cmp: impl Fn(usize) -> std::cmp::Ordering) -> i32 {
    use std::cmp::Ordering;
    let mut lo = 0usize;
    let mut step = n >> 1;
    loop {
        let i = lo + step;
        if i < n {
            match cmp(i) {
                Ordering::Equal => return i as i32,
                Ordering::Less => lo += step,
                Ordering::Greater => {}
            }
        }
        if step == 0 {
            break;
        }
        step >>= 1;
    }
    while lo < n {
        match cmp(lo) {
            Ordering::Equal => return lo as i32,
            Ordering::Greater => break,
            Ordering::Less => lo += 1,
        }
    }
    -(lo as i32 + 1)
}
