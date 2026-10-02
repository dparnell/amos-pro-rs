//! Strings in linear memory (see `layout::STR_*`).
//!
//! A string value is the absolute address of a block `[u32 length][bytes]`
//! (0 is the empty string). Blocks are allocated by bumping `STR_PTR` in
//! the current chunk, by the module itself (no call to the runtime) or by
//! the runtime for the strings it creates (results of keywords, Input...).
//! Chunks come from the array heap (`arrays.rs`): in the module's memory
//! natively, separate allocations on the web.
//!
//! Garbage collection is a compacting copy, done by the runtime only where
//! the module holds no string in its wasm locals (start of `run` and test
//! points, `Runtime::test`): every live string is copied into a new chunk
//! and the references are updated. The references are exactly the string
//! variables in memory (globals and the frames of running procedures), the
//! elements of string arrays in memory, and the table of string constants.

use std::collections::HashMap;

use super::*;
use crate::compiled::layout::ARR_DATA;

/// Size of a string chunk (larger strings get a chunk of their own size).
const CHUNK: u32 = 64 * 1024;

pub(super) struct Strings {
    /// Chunks in use: (address, size).
    chunks: Vec<(u32, u32)>,
    /// Bytes of chunks taken since the last collection.
    since_gc: u64,
    gc_limit: u64,
    /// Positions of the string constants of the program (their index is the
    /// slot in the constant table, `layout.consts`).
    pub consts: Vec<usize>,
}

impl Strings {
    pub fn new(consts: Vec<usize>) -> Strings {
        Strings { chunks: Vec::new(), since_gc: 0, gc_limit: 1 << 20, consts }
    }
}

/// Where a reference to a string is stored.
#[derive(Clone, Copy)]
enum Root {
    /// In the module's memory block (relative address).
    Mem(u32),
    /// In an array block (absolute address).
    Heap(u32),
}

impl Runtime {
    /// The bytes of the string at `a`.
    pub fn str_bytes<'m>(&self, mem: &'m mut [u8], a: i32) -> &'m [u8] {
        if a == 0 {
            return &[];
        }
        let len = {
            let b = self.heap.bytes(&mut *mem, self.base, a as u32, 4);
            u32::from_le_bytes(b[..4].try_into().unwrap())
        };
        self.heap.bytes(mem, self.base, a as u32 + 4, len)
    }

    pub fn str_of(&self, mem: &mut [u8], a: i32) -> AStr {
        astr(self.str_bytes(mem, a))
    }

    /// Takes a new chunk for at least `need` bytes; `false` if the memory
    /// must grow first (`Heap::grow_bytes`).
    fn new_chunk(&mut self, mem: &mut [u8], need: u32) -> bool {
        let size = need.max(CHUNK).next_multiple_of(16);
        let Some(addr) = self.heap.alloc(mem, self.base, size) else { return false };
        self.strings.chunks.push((addr, size));
        st_i32(mem, layout::STR_PTR, addr as i32);
        st_i32(mem, layout::STR_END, (addr + size) as i32);
        self.strings.since_gc += size as u64;
        if self.strings.since_gc > self.strings.gc_limit && !self.gc_wanted {
            self.gc_wanted = true;
            st_i32(mem, layout::ATT, 1);
        }
        true
    }

    /// A chunk for the module's allocator (`STR_PTR` was too close to
    /// `STR_END` for `need` bytes). Returns `ST_CONTINUE`, `ST_GROW` (the
    /// host grows the memory and calls again), or `ST_STOP` (no memory).
    pub fn str_chunk(&mut self, mem: &mut [u8], need: i32) -> i32 {
        if self.new_chunk(mem, need.max(0) as u32) {
            ST_CONTINUE
        } else if self.heap.kind == HeapKind::Linear {
            ST_GROW
        } else {
            self.trap_error = Some(errors::OUT_OF_MEMORY);
            ST_STOP
        }
    }

    /// Stores a string created by the runtime; returns its address.
    pub(super) fn alloc(&mut self, mem: &mut [u8], s: AStr) -> i32 {
        if s.is_empty() {
            return 0;
        }
        let size = (4 + s.len() as u32).next_multiple_of(4);
        let mut p = ld_i32(mem, layout::STR_PTR) as u32;
        if p as u64 + size as u64 > ld_i32(mem, layout::STR_END) as u32 as u64 {
            if !self.new_chunk(mem, size) {
                // The host keeps memory in reserve (`reserve_bytes`), so this
                // only happens when the memory cannot grow any more.
                self.set_pending(mem, Exc::Error(errors::OUT_OF_MEMORY));
                return 0;
            }
            p = ld_i32(mem, layout::STR_PTR) as u32;
        }
        let b = self.heap.bytes(mem, self.base, p, 4 + s.len() as u32);
        b[..4].copy_from_slice(&(s.len() as u32).to_le_bytes());
        b[4..].copy_from_slice(&s);
        st_i32(mem, layout::STR_PTR, (p + size) as i32);
        p as i32
    }

    /// The string constant number `idx` (token at `pos`): created once, then
    /// cached in the constant table.
    pub fn str_const(&mut self, mem: &mut [u8], pos: i32, idx: i32) -> i32 {
        let slot = self.layout.consts + idx as u32 * 4;
        let cur = ld_i32(mem, slot);
        if cur != 0 {
            return cur;
        }
        let p = pos as usize;
        let code = &self.prg.code;
        let n = structure::rd(code, p + 2) as usize;
        let s = astr(code.get(p + 4..p + 4 + n).unwrap_or(&[]));
        let h = self.alloc(mem, s);
        st_i32(mem, slot, h);
        h
    }

    /// Memory the host should add so that the runtime can allocate without
    /// growing during a call (native hosts check it after every call).
    pub fn reserve_bytes(&self, mem_len: usize) -> u32 {
        self.heap.reserve_bytes(mem_len)
    }

    fn root_get(&self, mem: &mut [u8], r: Root) -> i32 {
        match r {
            Root::Mem(a) => ld_i32(mem, a),
            Root::Heap(a) => i32::from_le_bytes(self.heap.bytes(mem, self.base, a, 4)[..4].try_into().unwrap()),
        }
    }

    fn root_set(&self, mem: &mut [u8], r: Root, v: i32) {
        match r {
            Root::Mem(a) => st_i32(mem, a, v),
            Root::Heap(a) => self.heap.bytes(mem, self.base, a, 4).copy_from_slice(&v.to_le_bytes()),
        }
    }

    /// Compacting collection (see the module documentation).
    pub(super) fn gc(&mut self, it: &Interp, mem: &mut [u8]) {
        self.gc_wanted = false;
        st_i32(mem, layout::ATT, it.vbl_pending as i32);
        // The references.
        let prg = self.prg.clone();
        let mut roots = Vec::new();
        for (i, d) in prg.globals.iter().enumerate() {
            if d.ty == 2 && structure::is_scalar(d) {
                roots.push(Root::Mem(self.layout.globals + i as u32 * 8));
            }
        }
        for (k, &idx) in self.frames.iter().enumerate() {
            let base = self.layout.frame_base(k + 1);
            if let Some(Ctl::Proc(f)) = it.ctl.get(idx) {
                for (i, d) in prg.procs[f.proc_index].locals.iter().enumerate() {
                    if d.ty == 2 && structure::is_scalar(d) {
                        roots.push(Root::Mem(base + i as u32 * 8));
                    }
                }
            }
        }
        for i in 0..self.strings.consts.len() as u32 {
            roots.push(Root::Mem(self.layout.consts + i * 4));
        }
        for (addr, count) in self.string_arrays() {
            for k in 0..count {
                roots.push(Root::Heap(addr + ARR_DATA + k * 4));
            }
        }
        // Live bytes, then copy them into one new chunk.
        let mut seen: HashMap<i32, i32> = HashMap::new();
        let mut live: u64 = 0;
        for &r in &roots {
            let v = self.root_get(mem, r);
            if v != 0 && seen.insert(v, 0).is_none() {
                live += (4 + self.str_bytes(mem, v).len() as u64).next_multiple_of(4);
            }
        }
        let old = std::mem::take(&mut self.strings.chunks);
        let size = (live as u32).saturating_add(CHUNK);
        self.strings.since_gc = 0;
        if !self.new_chunk(mem, size) {
            // No memory for the copy: keep everything (collect later).
            self.strings.chunks = old.into_iter().chain(self.strings.chunks.drain(..)).collect();
            return;
        }
        for &r in &roots {
            let v = self.root_get(mem, r);
            if v == 0 {
                continue;
            }
            let new = match seen.get(&v) {
                Some(&n) if n != 0 => n,
                _ => {
                    let s = self.str_of(mem, v);
                    let n = self.alloc(mem, s);
                    seen.insert(v, n);
                    n
                }
            };
            self.root_set(mem, r, new);
        }
        for (addr, size) in old {
            self.heap.release(self.base, addr, size);
        }
        self.strings.since_gc = 0;
        self.strings.gc_limit = (live * 2).max(1 << 20);
        self.gc_wanted = false;
        st_i32(mem, layout::ATT, it.vbl_pending as i32);
    }

    /// Bytes of string data in use (for tests).
    pub fn string_bytes(&self, mem: &[u8]) -> u64 {
        let ptr = ld_i32(mem, layout::STR_PTR) as u32;
        self.strings
            .chunks
            .iter()
            .map(|&(a, s)| if ptr >= a && ptr <= a + s { (ptr - a) as u64 } else { s as u64 })
            .sum()
    }
}
