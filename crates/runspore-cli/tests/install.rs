//! `install.sh` against signed release assets (spec/release.md): good assets install the
//! real `spore`; a tampered archive, a tampered `SHA256SUMS`, a foreign or missing
//! signature and a failing attestation are each refused with the install folder unchanged.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

use common::{Dir, BIN};
use sha2::{Digest, Sha256};

const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
];
const PENDING: &str = "ssh-ed25519 PENDING runspore-release";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn install_sh() -> String {
    std::fs::read_to_string(repo().join("install.sh")).unwrap()
}

/// The key line `install.sh` pins.
fn pinned_key(script: &str) -> &str {
    script
        .lines()
        .find_map(|l| l.strip_prefix("RELEASE_KEY='")?.strip_suffix('\''))
        .expect("install.sh has a RELEASE_KEY line")
}

fn ok(cmd: &mut Command) {
    let out = cmd.stdin(Stdio::null()).output().unwrap();
    assert!(
        out.status.success(),
        "{cmd:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `files` packed as a release archive: a gzip tar with each entry at its root.
fn pack(dir: &Path, name: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
    let staging = dir.join(format!("{name}-staging"));
    std::fs::create_dir_all(&staging).unwrap();
    for (entry, bytes) in files {
        let path = staging.join(entry);
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let archive = dir.join(format!("{name}.tar.gz"));
    let mut tar = Command::new("tar");
    tar.arg("-czf").arg(&archive).arg("-C").arg(&staging);
    tar.args(files.iter().map(|(entry, _)| entry));
    ok(&mut tar);
    std::fs::read(archive).unwrap()
}

/// The real `spore` as a release archive, packed once per test process.
fn spore_archive() -> &'static [u8] {
    static ARCHIVE: OnceLock<Vec<u8>> = OnceLock::new();
    ARCHIVE.get_or_init(|| {
        let dir = Dir::new("install-archive");
        let license = std::fs::read(repo().join("LICENSE")).unwrap();
        pack(
            &dir.path,
            "spore",
            &[
                ("spore", &std::fs::read(BIN).unwrap()),
                ("LICENSE", &license),
            ],
        )
    })
}

fn sums(archives: &[(String, Vec<u8>)]) -> String {
    let mut lines: Vec<String> = archives
        .iter()
        .map(|(name, bytes)| format!("{}  {name}\n", hex::encode(Sha256::digest(bytes))))
        .collect();
    lines.sort();
    lines.concat()
}

fn keygen(path: &Path) -> String {
    ok(Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            "runspore-release",
            "-f",
        ])
        .arg(path));
    let public = std::fs::read_to_string(path.with_extension("pub")).unwrap();
    public.trim().to_string()
}

/// A release folder signed with a throwaway key, a copy of `install.sh` pinning that
/// key, and a stand-in `gh` on the path.
struct Release {
    dir: Dir,
    assets: PathBuf,
    key: PathBuf,
}

impl Release {
    fn new(name: &str) -> Self {
        let dir = Dir::new(&format!("install-{name}"));
        let assets = dir.file("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let key = dir.file("release-key");
        let public = keygen(&key);
        let release = Self { dir, assets, key };
        release.pin(&public);
        let archives: Vec<(String, Vec<u8>)> = TARGETS
            .iter()
            .map(|t| (format!("spore-{t}.tar.gz"), spore_archive().to_vec()))
            .collect();
        release.publish(&archives);
        release.sign(&release.key.clone());

        let bin = release.dir.file("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let gh = bin.join("gh");
        std::fs::write(
            &gh,
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >>\"$GH_LOG\"\ncase $1 in\n\
             auth) exit \"$GH_AUTH_EXIT\" ;;\nattestation) exit \"$GH_VERIFY_EXIT\" ;;\nesac\nexit 2\n",
        )
        .unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        release
    }

    /// Writes the copy of `install.sh` with `key` as its pinned line.
    fn pin(&self, key: &str) {
        let script = install_sh();
        let line = format!("RELEASE_KEY='{}'", pinned_key(&script));
        let pinned = script.replace(&line, &format!("RELEASE_KEY='{key}'"));
        std::fs::write(self.dir.file("install.sh"), pinned).unwrap();
    }

    /// Writes the archives and their `SHA256SUMS`.
    fn publish(&self, archives: &[(String, Vec<u8>)]) {
        for (name, bytes) in archives {
            std::fs::write(self.assets.join(name), bytes).unwrap();
        }
        std::fs::write(self.assets.join("SHA256SUMS"), sums(archives)).unwrap();
    }

    fn sign(&self, key: &Path) {
        let _ = std::fs::remove_file(self.assets.join("SHA256SUMS.sig"));
        ok(Command::new("ssh-keygen")
            .args(["-q", "-Y", "sign", "-n", "runspore-release", "-f"])
            .arg(key)
            .arg(self.assets.join("SHA256SUMS")));
    }

    /// Every archive replaced by one whose `spore` is a shell script.
    fn evil_archives(&self) -> Vec<(String, Vec<u8>)> {
        let evil = pack(
            &self.dir.path,
            "evil",
            &[("spore", b"#!/bin/sh\necho evil\n")],
        );
        TARGETS
            .iter()
            .map(|t| (format!("spore-{t}.tar.gz"), evil.clone()))
            .collect()
    }

    fn install_dir(&self) -> PathBuf {
        self.dir.file("install")
    }

    fn gh_log(&self) -> String {
        std::fs::read_to_string(self.dir.file("gh.log")).unwrap_or_default()
    }

    /// Runs the pinned copy of `install.sh` with `gh` signed in and its check passing.
    fn install(&self, env: &[(&str, &str)]) -> Output {
        let path = format!(
            "{}:{}",
            self.dir.file("bin").display(),
            std::env::var("PATH").unwrap()
        );
        let mut cmd = Command::new("sh");
        cmd.arg(self.dir.file("install.sh"))
            .current_dir(&self.dir.path)
            .env("PATH", path)
            .env("HOME", self.dir.file("home"))
            .env("SPORE_INSTALL_DIR", self.install_dir())
            .env(
                "SPORE_DOWNLOAD_URL",
                format!("file://{}", self.assets.display()),
            )
            .env("GH_LOG", self.dir.file("gh.log"))
            .env("GH_AUTH_EXIT", "0")
            .env("GH_VERIFY_EXIT", "0")
            .env_remove("SPORE_VERSION")
            .stdin(Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    /// Runs `install.sh` over an earlier install and asserts it is refused with `why`,
    /// leaving that install as it was.
    fn refused(&self, env: &[(&str, &str)], why: &str) {
        let dir = self.install_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("spore"), "earlier").unwrap();
        let out = self.install(env);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.starts_with("install.sh: ")
                && stderr.lines().count() == 1
                && stderr.contains(why),
            "{stderr}"
        );
        let mut left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        left.sort();
        assert_eq!(left, ["spore"]);
        assert_eq!(
            std::fs::read_to_string(dir.join("spore")).unwrap(),
            "earlier"
        );
    }
}

fn version() -> String {
    let out = Command::new(BIN).arg("--version").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn good_assets_install_spore_and_it_runs() {
    let release = Release::new("good");
    let out = release.install(&[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("attestation checked"), "{stdout}");
    let installed = release.install_dir().join("spore");
    assert!(stdout.contains(&format!(
        "installed {} at {}",
        version(),
        installed.display()
    )));
    let ran = Command::new(&installed).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&ran.stdout).trim(), version());

    let log = release.gh_log();
    let verify = log.lines().find(|l| l.starts_with("attestation")).unwrap();
    assert!(
        verify.starts_with("attestation verify ")
            && verify.ends_with(".tar.gz --repo Web-tree/runspore"),
        "{log}"
    );
}

#[test]
fn installs_into_home_local_bin_by_default() {
    let release = Release::new("default-dir");
    let home = release.dir.file("home");
    let out = release.install(&[("SPORE_INSTALL_DIR", "")]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(home.join(".local/bin/spore").is_file());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("is not on PATH"), "{stdout}");
}

#[test]
fn without_gh_signed_in_the_attestation_is_skipped() {
    let release = Release::new("gh-signed-out");
    let out = release.install(&[("GH_AUTH_EXIT", "1")]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("attestation not checked"), "{stdout}");
    assert!(!release.gh_log().contains("attestation"));
}

#[test]
fn a_tampered_archive_is_refused() {
    let release = Release::new("tampered-archive");
    for (name, bytes) in release.evil_archives() {
        std::fs::write(release.assets.join(name), bytes).unwrap();
    }
    release.refused(&[], "does not match SHA256SUMS");
}

#[test]
fn a_tampered_sha256sums_is_refused() {
    let release = Release::new("tampered-sums");
    let sums = release.assets.join("SHA256SUMS");
    let text = std::fs::read_to_string(&sums).unwrap();
    let flipped = if text.starts_with('0') { "1" } else { "0" };
    std::fs::write(&sums, format!("{flipped}{}", &text[1..])).unwrap();
    release.refused(&[], "not signed by the runspore release key");
}

#[test]
fn sha256sums_rewritten_for_a_tampered_archive_is_refused() {
    let release = Release::new("tampered-both");
    release.publish(&release.evil_archives());
    release.refused(&[], "not signed by the runspore release key");
}

#[test]
fn a_signature_by_another_key_is_refused() {
    let release = Release::new("foreign-key");
    let other = release.dir.file("other-key");
    keygen(&other);
    release.sign(&other);
    release.refused(&[], "not signed by the runspore release key");
}

#[test]
fn a_missing_signature_is_refused() {
    let release = Release::new("no-signature");
    std::fs::remove_file(release.assets.join("SHA256SUMS.sig")).unwrap();
    release.refused(&[], "cannot download");
}

#[test]
fn a_failing_attestation_is_refused() {
    let release = Release::new("bad-attestation");
    release.refused(&[("GH_VERIFY_EXIT", "1")], "attestation");
}

#[test]
fn no_pinned_key_refuses_to_run() {
    let release = Release::new("no-key");
    release.pin(PENDING);
    release.refused(&[], "pins no release key");
}

#[test]
fn readme_and_spec_pin_the_key_install_sh_pins() {
    let script = install_sh();
    let key = pinned_key(&script);
    for doc in ["README.md", "spec/release.md"] {
        let text = std::fs::read_to_string(repo().join(doc)).unwrap();
        assert!(
            text.lines().any(|l| l.trim() == key),
            "{doc} does not show the release key line of install.sh: {key}"
        );
    }
}
