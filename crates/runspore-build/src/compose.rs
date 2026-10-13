//! Encodes a composed artifact (`spec/artifact.md` §7): one component holding the
//! kernel component and a workflow artifact, each nested unchanged, instantiated side
//! by side, with both interfaces exported again under their own names.
//!
//! Neither component imports anything, so the composition wires nothing: it is the
//! two nested components, two instantiations with no arguments, two aliases and two
//! exports. The nested bytes are the inputs byte for byte, so a host that extracts the
//! kernel finds the certified build.

use wasm_encoder::{
    Alias, Component, ComponentAliasSection, ComponentExportKind, ComponentExportSection,
    ComponentInstanceSection, ComponentSectionId, RawSection,
};

/// The interface the kernel exports (`contracts/wit/machine/machine.wit`).
pub const MACHINE_INTERFACE: &str = "runspore:machine/reducer@0.1.0";
/// The interface a workflow artifact exports (`contracts/wit/artifact/artifact.wit`).
pub const ARTIFACT_INTERFACE: &str = "runspore:artifact/reader@0.1.0";

/// Component bytes that nest `kernel` and `artifact` as they are and export
/// [`MACHINE_INTERFACE`] from the first and [`ARTIFACT_INTERFACE`] from the second;
/// deterministic and unchecked. A host checks the result when it loads it: a nested
/// component without its interface makes the composition invalid.
pub fn compose(kernel: &[u8], artifact: &[u8]) -> Result<Vec<u8>, String> {
    // Section sizes are 32-bit; the encoder would panic past that, so refuse first.
    if [kernel.len(), artifact.len()]
        .iter()
        .any(|len| u32::try_from(*len).is_err())
    {
        return Err("the composed artifact does not fit in 4 GiB".to_string());
    }
    fn nested(bytes: &[u8]) -> RawSection<'_> {
        RawSection {
            id: ComponentSectionId::Component.into(),
            data: bytes,
        }
    }
    let no_args = Vec::<(&str, ComponentExportKind, u32)>::new;
    let mut instances = ComponentInstanceSection::new();
    instances.instantiate(0, no_args());
    instances.instantiate(1, no_args());
    let mut aliases = ComponentAliasSection::new();
    aliases.alias(Alias::InstanceExport {
        instance: 0,
        kind: ComponentExportKind::Instance,
        name: MACHINE_INTERFACE,
    });
    aliases.alias(Alias::InstanceExport {
        instance: 1,
        kind: ComponentExportKind::Instance,
        name: ARTIFACT_INTERFACE,
    });
    // Instances 0 and 1 are the two instantiations; the aliases are instances 2 and 3.
    let mut exports = ComponentExportSection::new();
    exports.export(MACHINE_INTERFACE, ComponentExportKind::Instance, 2, None);
    exports.export(ARTIFACT_INTERFACE, ComponentExportKind::Instance, 3, None);

    let mut component = Component::new();
    component
        .section(&nested(kernel))
        .section(&nested(artifact))
        .section(&instances)
        .section(&aliases)
        .section(&exports);
    Ok(component.finish())
}
