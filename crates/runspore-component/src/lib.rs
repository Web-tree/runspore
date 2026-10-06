//! The Runspore kernel as a WebAssembly component exporting world `machine`.
//!
//! Built for `wasm32-unknown-unknown` and componentised by `runspore-wasmtime`'s
//! build script; the component has no imports. Every export converts the WIT records
//! field for field to `runspore_types::reducer` and calls `runspore_kernel`. On any
//! other target the crate is empty.

#[cfg(target_arch = "wasm32")]
mod guest;

#[cfg(all(target_arch = "wasm32", feature = "echo"))]
mod echo;
