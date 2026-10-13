//! Encodes a workflow artifact: an import-free component of the `runspore:artifact`
//! world (`contracts/wit/artifact/artifact.wit`) whose two exports return bytes held
//! in its data segment.
//!
//! The core module is written instruction by instruction, so building an artifact
//! needs no compiler. Its linear memory holds, from `TABLE` on: one 16-byte row per
//! blob (`digest ptr, digest len, bytes ptr, bytes len`), the manifest, then each
//! digest string and its bytes. `blob` compares the digest it is given with each row;
//! `cabi_realloc` is a bump allocator for that argument.

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, Instruction as I, MemArg, MemorySection,
    MemoryType, Module, TypeSection, ValType,
};
use wit_component::{ComponentEncoder, StringEncoding};
use wit_parser::Resolve;

const WIT: &str = include_str!("../../../contracts/wit/artifact/artifact.wit");
const INTERFACE: &str = "runspore:artifact/reader@0.1.0";

/// The 12-byte return area of both exports.
const RET: i32 = 16;
/// Where the blob table starts.
const TABLE: u32 = 64;
const ROW: u32 = 16;
const PAGE: u64 = 65_536;

/// Component bytes that return `manifest` and `blobs` (digest, bytes) as they are,
/// unchecked; deterministic. [`crate::Built::seal`] is the checked way to an artifact.
pub fn encode(manifest: &[u8], blobs: &[(String, &[u8])]) -> Result<Vec<u8>, String> {
    let mut module = core_module(manifest, blobs)?;
    let mut resolve = Resolve::default();
    let package = resolve
        .push_str("artifact.wit", WIT)
        .map_err(|e| format!("artifact WIT: {e:#}"))?;
    let world = resolve
        .select_world(&[package], Some("artifact"))
        .map_err(|e| format!("artifact WIT: {e:#}"))?;
    wit_component::embed_component_metadata(
        &mut module,
        &resolve,
        world,
        StringEncoding::UTF8,
        false,
    )
    .map_err(|e| format!("artifact metadata: {e:#}"))?;
    ComponentEncoder::default()
        .validate(true)
        .module(&module)
        .and_then(|encoder| encoder.encode())
        .map_err(|e| format!("artifact component: {e:#}"))
}

fn core_module(manifest: &[u8], blobs: &[(String, &[u8])]) -> Result<Vec<u8>, String> {
    let too_large = || "the artifact does not fit in 4 GiB".to_string();
    let rows = u32::try_from(blobs.len()).map_err(|_| too_large())?;
    let mut data: Vec<u8> = vec![0; (rows * ROW) as usize];
    let mut place = |bytes: &[u8]| -> Result<(i32, i32), String> {
        let at = u64::from(TABLE) + data.len() as u64;
        data.extend_from_slice(bytes);
        let ptr = i32::try_from(at).map_err(|_| too_large())?;
        let len = i32::try_from(bytes.len()).map_err(|_| too_large())?;
        Ok((ptr, len))
    };
    let (manifest_ptr, manifest_len) = place(manifest)?;
    let mut table = Vec::with_capacity(blobs.len());
    for (digest, bytes) in blobs {
        let (digest_ptr, digest_len) = place(digest.as_bytes())?;
        let (bytes_ptr, bytes_len) = place(bytes)?;
        table.push([digest_ptr, digest_len, bytes_ptr, bytes_len]);
    }
    for (i, row) in table.iter().enumerate() {
        for (j, word) in row.iter().enumerate() {
            let at = i * ROW as usize + j * 4;
            data[at..at + 4].copy_from_slice(&word.to_le_bytes());
        }
    }
    let heap = (u64::from(TABLE) + data.len() as u64 + 7) & !7;
    let heap = i32::try_from(heap).map_err(|_| too_large())?;
    let pages = (heap as u64).div_ceil(PAGE) + 1;

    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I32]);
    types
        .ty()
        .function([ValType::I32, ValType::I32], [ValType::I32]);
    types.ty().function([ValType::I32; 4], [ValType::I32]);
    let mut functions = FunctionSection::new();
    functions.function(0).function(1).function(2);
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: pages,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(heap),
    );
    let mut exports = ExportSection::new();
    exports
        .export("memory", ExportKind::Memory, 0)
        .export(&format!("{INTERFACE}#manifest"), ExportKind::Func, 0)
        .export(&format!("{INTERFACE}#blob"), ExportKind::Func, 1)
        .export("cabi_realloc", ExportKind::Func, 2);
    let mut code = CodeSection::new();
    code.function(&manifest_fn(manifest_ptr, manifest_len));
    code.function(&blob_fn(rows));
    code.function(&realloc_fn());
    let mut segments = DataSection::new();
    segments.active(0, &ConstExpr::i32_const(TABLE as i32), data);

    let mut module = Module::new();
    module
        .section(&types)
        .section(&functions)
        .section(&memories)
        .section(&globals)
        .section(&exports)
        .section(&code)
        .section(&segments);
    Ok(module.finish())
}

fn mem(offset: u64, align: u32) -> MemArg {
    MemArg {
        offset,
        align,
        memory_index: 0,
    }
}

fn emit(f: &mut Function, instructions: &[I<'_>]) {
    for instruction in instructions {
        f.instruction(instruction);
    }
}

/// `manifest() -> list<u8>`: writes `(ptr, len)` to the return area.
fn manifest_fn(ptr: i32, len: i32) -> Function {
    let mut f = Function::new([]);
    emit(
        &mut f,
        &[
            I::I32Const(RET),
            I::I32Const(ptr),
            I::I32Store(mem(0, 2)),
            I::I32Const(RET),
            I::I32Const(len),
            I::I32Store(mem(4, 2)),
            I::I32Const(RET),
            I::End,
        ],
    );
    f
}

/// `blob(digest: string) -> option<list<u8>>`, with the string at params 0 and 1.
/// Locals: 2 the row index, 3 the row address, 4 the byte index.
fn blob_fn(rows: u32) -> Function {
    let mut f = Function::new([(3, ValType::I32)]);
    emit(
        &mut f,
        &[
            I::Block(BlockType::Empty),
            I::Loop(BlockType::Empty),
            // No row left: not found.
            I::LocalGet(2),
            I::I32Const(rows as i32),
            I::I32GeU,
            I::BrIf(1),
            I::LocalGet(2),
            I::I32Const(ROW as i32),
            I::I32Mul,
            I::I32Const(TABLE as i32),
            I::I32Add,
            I::LocalSet(3),
            // Same length as the row's digest: compare byte by byte.
            I::LocalGet(3),
            I::I32Load(mem(4, 2)),
            I::LocalGet(1),
            I::I32Eq,
            I::If(BlockType::Empty),
            I::I32Const(0),
            I::LocalSet(4),
            I::Block(BlockType::Empty),
            I::Loop(BlockType::Empty),
            I::LocalGet(4),
            I::LocalGet(1),
            I::I32GeU,
            I::If(BlockType::Empty),
            // Every byte matched: some((ptr, len)).
            I::I32Const(RET),
            I::I32Const(1),
            I::I32Store8(mem(0, 0)),
            I::I32Const(RET),
            I::LocalGet(3),
            I::I32Load(mem(8, 2)),
            I::I32Store(mem(4, 2)),
            I::I32Const(RET),
            I::LocalGet(3),
            I::I32Load(mem(12, 2)),
            I::I32Store(mem(8, 2)),
            I::I32Const(RET),
            I::Return,
            I::End,
            I::LocalGet(3),
            I::I32Load(mem(0, 2)),
            I::LocalGet(4),
            I::I32Add,
            I::I32Load8U(mem(0, 0)),
            I::LocalGet(0),
            I::LocalGet(4),
            I::I32Add,
            I::I32Load8U(mem(0, 0)),
            I::I32Ne,
            I::BrIf(1),
            I::LocalGet(4),
            I::I32Const(1),
            I::I32Add,
            I::LocalSet(4),
            I::Br(0),
            I::End,
            I::End,
            I::End,
            // Next row.
            I::LocalGet(2),
            I::I32Const(1),
            I::I32Add,
            I::LocalSet(2),
            I::Br(0),
            I::End,
            I::End,
            // none
            I::I32Const(RET),
            I::I32Const(0),
            I::I32Store8(mem(0, 0)),
            I::I32Const(RET),
            I::End,
        ],
    );
    f
}

/// `cabi_realloc(old_ptr, old_size, align, new_size)`: a bump allocator from the
/// global, growing memory when needed. The canonical ABI only ever asks it for fresh
/// memory here; a reallocation traps. Locals: 4 the new pointer, 5 its end.
fn realloc_fn() -> Function {
    let mut f = Function::new([(2, ValType::I32)]);
    emit(
        &mut f,
        &[
            I::LocalGet(0),
            I::If(BlockType::Empty),
            I::Unreachable,
            I::End,
            // ptr = (bump + align - 1) & -align
            I::GlobalGet(0),
            I::LocalGet(2),
            I::I32Add,
            I::I32Const(1),
            I::I32Sub,
            I::I32Const(0),
            I::LocalGet(2),
            I::I32Sub,
            I::I32And,
            I::LocalTee(4),
            I::LocalGet(3),
            I::I32Add,
            I::LocalSet(5),
            I::LocalGet(5),
            I::MemorySize(0),
            I::I32Const(16),
            I::I32Shl,
            I::I32GtU,
            I::If(BlockType::Empty),
            I::LocalGet(5),
            I::MemorySize(0),
            I::I32Const(16),
            I::I32Shl,
            I::I32Sub,
            I::I32Const(65_535),
            I::I32Add,
            I::I32Const(16),
            I::I32ShrU,
            I::MemoryGrow(0),
            I::I32Const(-1),
            I::I32Eq,
            I::If(BlockType::Empty),
            I::Unreachable,
            I::End,
            I::End,
            I::LocalGet(5),
            I::GlobalSet(0),
            I::LocalGet(4),
            I::End,
        ],
    );
    f
}
