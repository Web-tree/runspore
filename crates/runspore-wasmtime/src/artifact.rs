//! Loads workflow artifacts (`spec/artifact.md` §5): untrusted components of the
//! `runspore:artifact` world, run in the kernel's sandbox with only `manifest` and
//! `blob` ever called.

use std::sync::OnceLock;

use runspore_types::artifact::{Artifact, ArtifactError, Manifest};
use runspore_types::digest;
use wasmtime::component::{Component, Linker};
use wasmtime::{Engine, Store};

use crate::{deterministic_config, new_store, Bounds, Guards};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../contracts/wit/artifact",
        world: "artifact",
    });
}

/// Bounds for one artifact instance: its data segment holds every file of the
/// workflow, so memory is larger than the kernel's.
const GUARDS: Guards = Guards {
    fuel: 1_000_000_000,
    max_memory_bytes: 1024 * 1024 * 1024,
};

fn engine() -> Result<&'static Engine, ArtifactError> {
    static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| Engine::new(&deterministic_config()).map_err(|e| format!("{e:#}")))
        .as_ref()
        .map_err(|e| ArtifactError::invalid(format!("artifact engine: {e}")))
}

fn invalid(error: &wasmtime::Error) -> ArtifactError {
    ArtifactError::invalid(format!("{error:#}"))
}

/// One instance of an artifact, for reading its manifest and blobs.
pub struct ArtifactReader {
    store: Store<Bounds>,
    artifact: bindings::Artifact,
}

impl ArtifactReader {
    /// Compiles and instantiates the component with no imports. Anything that is not
    /// a component of the `runspore:artifact` world is `artifact.invalid`.
    pub fn open(bytes: &[u8]) -> Result<Self, ArtifactError> {
        let engine = engine()?;
        let component = Component::new(engine, bytes).map_err(|e| invalid(&e))?;
        let linker = Linker::<Bounds>::new(engine);
        let mut store = new_store(engine, GUARDS).map_err(|e| invalid(&e))?;
        let artifact = bindings::Artifact::instantiate(&mut store, &component, &linker)
            .map_err(|e| invalid(&e))?;
        Ok(Self { store, artifact })
    }

    /// The raw bytes `manifest()` returns.
    pub fn manifest(&mut self) -> Result<Vec<u8>, ArtifactError> {
        self.artifact
            .runspore_artifact_reader()
            .call_manifest(&mut self.store)
            .map_err(|e| invalid(&e))
    }

    /// The bytes `blob(digest)` returns, unchecked.
    pub fn blob(&mut self, digest: &str) -> Result<Option<Vec<u8>>, ArtifactError> {
        self.artifact
            .runspore_artifact_reader()
            .call_blob(&mut self.store, digest)
            .map_err(|e| invalid(&e))
    }

    /// The bytes of a manifest entry, checked against its digest.
    pub fn entry(&mut self, entry_digest: &str) -> Result<Vec<u8>, ArtifactError> {
        let bytes = self.blob(entry_digest)?.ok_or_else(|| {
            ArtifactError::invalid(format!("the artifact holds no blob {entry_digest}"))
        })?;
        if digest::blob(&bytes) != entry_digest {
            return Err(ArtifactError::new(
                "artifact.digest-mismatch",
                format!("blob {entry_digest} does not match its digest"),
            ));
        }
        Ok(bytes)
    }
}

/// The load sequence of `spec/artifact.md` §5 steps 1–3: instantiate, read and check
/// the manifest, and read the package with its digest checked.
pub fn load_artifact(bytes: &[u8]) -> Result<Artifact, ArtifactError> {
    let mut reader = ArtifactReader::open(bytes)?;
    let manifest = Manifest::decode(&reader.manifest()?)?;
    let package = reader.blob(&manifest.main)?.ok_or_else(|| {
        ArtifactError::invalid(format!("the artifact holds no package {}", manifest.main))
    })?;
    if digest::package(&package) != manifest.main {
        return Err(ArtifactError::new(
            "artifact.digest-mismatch",
            format!("the package does not match its digest {}", manifest.main),
        ));
    }
    Ok(Artifact {
        digest: digest::artifact(bytes),
        bytes: bytes.to_vec(),
        manifest,
        package,
    })
}
