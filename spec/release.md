# Releases and `install.sh`

Applies from `spore` 0.2.0. Every release of `spore` on GitHub publishes the same set
of assets, signed with one **release key**. `install.sh` downloads them, checks them
and installs `spore`. Any other installer follows the same checks (§5); the asset
names, the checksum format and the signature format below are its contract.

## 1. Assets

A release is the tag `v<version>`, where `<version>` is the workspace version in
`Cargo.toml` and what `spore --version` prints after `spore `. Its assets:

| Asset | Holds |
| --- | --- |
| `spore-aarch64-apple-darwin.tar.gz` | macOS, arm64 |
| `spore-x86_64-apple-darwin.tar.gz` | macOS, x86_64 |
| `spore-aarch64-unknown-linux-musl.tar.gz` | Linux, arm64, statically linked |
| `spore-x86_64-unknown-linux-musl.tar.gz` | Linux, x86_64, statically linked |
| `SHA256SUMS` | The checksum of every archive (§2) |
| `SHA256SUMS.sig` | The signature of `SHA256SUMS` made with the release key (§3) |

Asset names carry no version, so the newest release's assets are always at
`https://github.com/Web-tree/runspore/releases/latest/download/<asset>`, and a given
release's at `https://github.com/Web-tree/runspore/releases/download/v<version>/<asset>`.

Each archive is a gzip-compressed tar holding exactly two entries at its root, with no
directory: `spore` (the binary, mode 0755) and `LICENSE`.

Each archive also has a build-provenance attestation in GitHub's attestation store,
made by the release workflow from the build that produced it. It is not a file
asset. `gh attestation verify <archive> --repo Web-tree/runspore` checks it.

## 2. `SHA256SUMS`

One line per archive, and no other lines: the SHA-256 of the archive as 64 lowercase
hexadecimal digits, two spaces, the asset name, and `\n`. Lines are sorted by asset
name. This is the output of `sha256sum spore-*.tar.gz`. A verifier finds the line
whose name field equals the asset name exactly; none, or more than one, is a failure.

## 3. `SHA256SUMS.sig`

An OpenSSH signature (the `SSHSIG` format of OpenSSH's `PROTOCOL.sshsig`) over the
exact bytes of `SHA256SUMS`, made with the release key, with:

- key type `ssh-ed25519`;
- namespace `runspore-release`;
- hash algorithm `sha512`.

`ssh-keygen -Y sign -f <key> -n runspore-release SHA256SUMS` makes it, and
`ssh-keygen -Y verify` checks it (§5). A verifier without `ssh-keygen` checks it with
any Ed25519 implementation:

1. Take the base64 text between `-----BEGIN SSH SIGNATURE-----` and
   `-----END SSH SIGNATURE-----` and decode it.
2. Read, in order: the 6 bytes `SSHSIG`; a big-endian `uint32` version, which is 1;
   then five SSH `string`s (a big-endian `uint32` length, then that many bytes):
   public key, namespace, reserved, hash algorithm, signature. Nothing may follow.
3. Refuse unless the public key equals the decoded base64 field of the pinned key
   line (§4), the namespace is `runspore-release`, the reserved string is empty, the
   hash algorithm is `sha512`, and the signature holds two strings: `ssh-ed25519` and
   a 64-byte Ed25519 signature.
4. The signed message is `SSHSIG`, then the strings namespace, reserved and hash
   algorithm, then the string holding the 64-byte SHA-512 of `SHA256SUMS`.
5. The 32-byte Ed25519 public key is the second string inside the public key field
   (the first is `ssh-ed25519`). Verify the 64-byte signature over the message from
   step 4 with it.

## 4. The release key

The public half, as one OpenSSH public key line:

```text
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGy3mjvtp10UZgB9Y8u3KNBOh66DHPo3bdbZEszh9b1k runspore-release
```

When the line reads `PENDING`, no key has been made yet: no release is published, and
`install.sh` refuses to run.

The same line is pinned in `install.sh` (`RELEASE_KEY`), in the README, and in every
other installer. A test fails when the README or this file differs from `install.sh`.

The private half exists only as the repository secret `RELEASE_SIGNING_KEY`, which
the release workflow alone reads. `scripts/new-release-key.sh` makes a key pair,
stores the private half in that secret, and writes the new public line into
`install.sh`, the README and this file. The same script rotates the key. After a
rotation, releases signed with the old key no longer install with the new
`install.sh`, and every other installer needs a release that pins the new line.

## 5. Checks before anything runs

Nothing runs or installs a downloaded `spore` before both checks pass:

1. `SHA256SUMS.sig` is a valid signature of the downloaded `SHA256SUMS` by the pinned
   release key (§3).
2. The SHA-256 of the downloaded archive equals its line in that `SHA256SUMS` (§2).

The checksum alone is not enough: whoever can replace an archive in a release can
replace `SHA256SUMS` too. The signature alone is not enough either, since it covers
the archive only through the checksum.

Checking the attestation (§1) is a third, optional check. It ties the archive to the
repository and the workflow run that built it, but it trusts GitHub's identity
service where the first two trust only the pinned key.

## 6. `install.sh`

A POSIX `sh` script at the repository root, run as

```sh
curl -fsSL https://raw.githubusercontent.com/Web-tree/runspore/main/install.sh | sh
```

It needs `curl`, `tar`, `ssh-keygen` (OpenSSH 8.1 or newer) and `sha256sum` or
`shasum`. In order, it:

1. Refuses to run when its pinned key is not an `ssh-ed25519` key line.
2. Picks the archive for the machine from `uname -s` and `uname -m`: `Darwin` or
   `Linux`, and `arm64`/`aarch64` or `x86_64`/`amd64`. Any other pair is refused.
   On macOS, a shell translated by Rosetta (`sysctl.proc_translated` is 1) gets the
   arm64 archive.
3. Downloads the archive, `SHA256SUMS` and `SHA256SUMS.sig` into a fresh temporary
   directory, which it removes on exit.
4. Makes both checks of §5 with the pinned key.
5. When `gh` is on the path and `gh auth status` succeeds, runs
   `gh attestation verify <archive> --repo Web-tree/runspore`, and refuses when it
   fails. Otherwise it says the attestation was not checked.
6. Extracts only `spore` and moves it into the install folder, creating the folder
   when missing. The move replaces an existing `spore` in one rename, so the folder
   never holds a partial file.
7. Prints what `spore --version` prints. It warns when the install folder is not on
   `PATH`, or when `command -v spore` finds another `spore` first.

Any failure prints one line starting `install.sh: ` to standard error and exits 1,
with the install folder unchanged.

| Variable | Default | Meaning |
| --- | --- | --- |
| `SPORE_VERSION` | newest release | The release to install, `0.2.0` or `v0.2.0` |
| `SPORE_INSTALL_DIR` | `$HOME/.local/bin` | The install folder |
| `SPORE_DOWNLOAD_URL` | the release's download URL (§1) | A folder holding the assets, for a mirror; overrides `SPORE_VERSION` |

No variable turns a check of §5 off. A mirror is safe for that reason: its files pass
only when the release key signed them.

## 7. Building from source

`cargo install --git https://github.com/Web-tree/runspore --locked runspore-cli`
builds and installs `spore` from the repository, with no signature checks.

## 8. The release cycle

### The release PR

On every push to `main`, `.github/workflows/release-please.yml` runs release-please
with `release-please-config.json` and `.release-please-manifest.json`. It keeps one
release PR open, which bumps the workspace version in `Cargo.toml`, the version of
every workspace package in `Cargo.lock`, and the manifest, and adds the merged
changes to `CHANGELOG.md`. Each merge to `main` updates that PR. Before 1.0, a `feat`
bumps the minor version and a `fix` the patch; a `!` (breaking change) bumps the
minor version too. The first release is `0.2.0`: `release-as` in the config pins it,
and is removed once 0.2.0 is out.

release-please runs as the org's `webtree-release` GitHub App, through the shared
workflow `Web-tree/actions/.github/workflows/release-please.yml`. A release PR pushed
with `GITHUB_TOKEN` would get its `pull_request` checks only after someone approves
them; pushed by the App, it gets them like any other PR. The job reads the App's
credentials from AWS SSM through the role in `AWS_ROLE`, a secret of the `release`
environment, which only `main` may deploy to.

Merging the release PR is the release: the push of that merge makes release-please
tag `v<version>` and create the GitHub release, with the PR's changelog as its notes.

### The release workflow

`.github/workflows/release.yml` is a reusable workflow. `release-please.yml` calls it
on every push to `main`: with the new tag when release-please made a release, with no
tag otherwise, so the release is built in the run that made it. The push of the tag
the App made starts `release.yml` too, and that run skips every job. A tag `v*`
pushed by hand also starts `release.yml`, for a release that has to be made again.

With no tag, it builds each archive of §1 on a runner of its operating system and
keeps them as workflow artifacts for 7 days; it signs and publishes nothing. A target
that fails to build fails the run, so a broken target shows the day it breaks, not on
release day.

With a tag, it builds the archives from the tagged commit and:

1. stops if the tag's release already has assets, before it changes anything;
2. writes `SHA256SUMS` and signs it with `RELEASE_SIGNING_KEY`;
3. makes the build-provenance attestations of the archives;
4. runs `install.sh` against the assets with the pinned key, the attestation check
   included, and fails unless the installed `spore --version` names the tag's
   version;
5. uploads the six assets to the tag's release, or creates the release with them when
   there is none (a tag pushed by hand).

A failure at any step uploads nothing. A release release-please made stays without
assets, and `install.sh` cannot install it, until a run for its tag succeeds. To make
a release again, delete its assets (or the release) and push the tag again.

### Commit titles

release-please reads conventional commits, and a squash merge makes the PR title the
commit title. So a PR title is `<type>: <subject>` or `<type>(<scope>): <subject>`,
with `<type>` one of `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`,
`chore`, `revert`, and `!` before the `:` for a breaking change. The `ticket` workflow
titles its PRs `<type>(<ticket ID>): <ticket title>`, with `feat` or `fix` from its
implement step's result, for example
`feat(R4): workflow artifacts: spore build, load, pin and inspect`.

`.github/workflows/pr-title.yml` checks every PR's title with
`scripts/check-pr-title.sh`. A title of any other form fails the check, which names
the expected form, when the repository variable `PR_TITLE_STRICT` is `true`;
otherwise it only warns and passes. The variable is set once the 0.2 build runs a
`spore` whose `ticket` workflow writes conventional titles: until then it ships PRs
titled `R5: …`, and its ship step stops on any failed check.

### The release key

The key is made once, by a repository admin, with `scripts/new-release-key.sh` (§4),
and the three files it changes are merged before the first release. The release PR
cannot be merged before that: its run would fail at signing, and `install.sh` on
`main` refuses to run while it pins no key.

## 9. Required tests

Run on Linux and macOS on every change, against a real `spore` archive signed with a
throwaway key pinned into a copy of `install.sh`:

- good assets install `spore` into `SPORE_INSTALL_DIR`, and into `$HOME/.local/bin`
  by default, and the installed binary runs;
- each of these is refused with the install folder unchanged: a tampered archive; a
  tampered `SHA256SUMS`, including one that matches a tampered archive; a signature
  by another key; a missing signature; a failing attestation check when `gh` is
  present and signed in;
- `install.sh` with no key pinned refuses to run;
- the pinned line in the README and in §4 equals the one in `install.sh`.
