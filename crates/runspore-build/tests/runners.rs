//! The build's check of a `component` action's step runner (spec/host.md §3,
//! `component`; spec/dot.md §4), and the small runners the catalogue's examples hold,
//! made here from `contracts/wit/runner/runner.wit` with wit-component's dummy module
//! so their bytes can be made again.
//!
//! `cargo test -p runspore-build --test runners -- --ignored bless` rewrites them.

use std::path::{Path, PathBuf};

use runspore_build::runner::{self, Problem};
use wit_component::{ComponentEncoder, StringEncoding};
use wit_parser::{ManglingAndAbi, Resolve};

const WIT: &str = include_str!("../../../contracts/wit/runner/runner.wit");

/// A WASI-like interface no host world offers to a runner.
const CLOCK: &str = "package wasi:clocks@0.2.0;
interface wall-clock { now: func() -> u64; }";

const WITH_CLOCK: &str = "package example:runners;
world with-clock {
  import runspore:runner/host@0.1.1;
  import wasi:clocks/wall-clock@0.2.0;
  export runspore:runner/step@0.1.1;
}";

/// A component of `world` whose functions do nothing: its imports and exports are the
/// world's.
fn dummy(world: &str) -> Vec<u8> {
    let mut resolve = Resolve::default();
    let runner = resolve.push_str("runner.wit", WIT).unwrap();
    resolve.push_str("clock.wit", CLOCK).unwrap();
    let examples = resolve.push_str("with-clock.wit", WITH_CLOCK).unwrap();
    let world = match world {
        "runner" => resolve.select_world(&[runner], Some("runner")).unwrap(),
        other => resolve.select_world(&[examples], Some(other)).unwrap(),
    };
    let mut module = wit_component::dummy_module(&resolve, world, ManglingAndAbi::Standard32);
    wit_component::embed_component_metadata(
        &mut module,
        &resolve,
        world,
        StringEncoding::UTF8,
        false,
    )
    .unwrap();
    ComponentEncoder::default()
        .validate(true)
        .module(&module)
        .and_then(|encoder| encoder.encode())
        .unwrap()
}

fn catalogue() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("catalogue")
}

/// Each runner of an example or of its fixed folder, and the world it is made from.
fn made() -> Vec<(PathBuf, Vec<u8>)> {
    let mut made = vec![
        (
            catalogue().join("examples/component.grant-unsupported/runner.wasm"),
            dummy("runner"),
        ),
        (
            catalogue().join("examples/component.import-unsupported/runner.wasm"),
            dummy("with-clock"),
        ),
    ];
    for code in [
        "component.no-module",
        "component.invalid",
        "component.import-unsupported",
    ] {
        let path = catalogue().join("fixed").join(code).join("runner.wasm");
        made.push((path, dummy("runner")));
    }
    made
}

#[test]
fn a_runner_of_the_world_passes() {
    assert_eq!(runner::check(&dummy("runner")), Ok(()));
}

#[test]
fn a_runner_built_for_0_1_0_passes() {
    let old = include_bytes!("../../runspore-host/tests/fixtures/test-runner-0.1.0.wasm");
    assert_eq!(runner::check(old), Ok(()));
}

#[test]
fn an_import_outside_the_world_is_named() {
    assert_eq!(
        runner::check(&dummy("with-clock")),
        Err(Problem::Import("wasi:clocks/wall-clock@0.2.0".into()))
    );
}

#[test]
fn a_file_that_is_no_runner_is_invalid() {
    for bytes in [&b"not wasm"[..], b"\0asm\x01\0\0\0"] {
        assert!(
            matches!(runner::check(bytes), Err(Problem::Invalid(_))),
            "{bytes:?}"
        );
    }
}

#[test]
fn the_example_runners_are_made_from_the_world() {
    for (path, bytes) in made() {
        let kept = std::fs::read(&path).unwrap_or_default();
        assert!(
            kept == bytes,
            "{} is not what the world makes; run the bless test",
            path.display()
        );
    }
}

#[test]
#[ignore = "rewrites the catalogue's example runners"]
fn bless() {
    for (path, bytes) in made() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
}
