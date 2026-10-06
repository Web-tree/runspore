# A shorter CLI name for the Runspore binary

**Recommendation: rename the binary to `spore`.** No packaged executable, shell alias or base-system tool uses that name in any source checked. The crates.io, npm and PyPI packages named `spore` are old; the crate and the npm package install no `spore` command, and the PyPI one was last uploaded in 2014.

**Runner-up: `rnsp`.** It is free in every source checked, including all three registries, but it is harder to say and remember than `spore`.

Checked on 2026-10-06. Product, domain and package names stay `runspore`. Only the installed command changes.

## Candidates checked

"free" means the source has no package, executable or alias with that exact name. Each link goes to the query result. Full commands are in [Method](#method).

| Name | Homebrew | Debian / Ubuntu | Arch | Nix | crates.io | npm | PyPI | Shell / aliases | GitHub |
|---|---|---|---|---|---|---|---|---|---|
| `rs` | [free][brew-api] | [`/usr/bin/rs` from package `rs`][deb-rs] | [free][arch-rs] | [package `rs` provides `rs`][nix] | [free][cr-rs] | [taken, no bin][npm-rs] | [taken][py-rs] | `/usr/bin/rs` on macOS ("reshape a data array", `man rs`). oh-my-zsh: [`rs='repo sync'`][omz-repo], [`rs='rails server'`][omz-rails-rs] | not counted: 431,416 repos match the name |
| `sp` | [free][brew-api] | [no `/usr/bin/sp` in stable][deb-sp]; sid unclear (results page did not parse) | [free][arch-sp] | [only `rPackages.sp`, no program][nix] | [taken, bin `sp`][cr-sp] | [taken, bin `sp`][npm-sp] | [taken][py-sp] | oh-my-zsh: [web-search][omz-ws], [singlechar][omz-sc], [rails][omz-rails-sp] all alias `sp` | not counted: 5.1M repos match |
| `rsp` | [free][brew-api] | [free][deb-rsp] | [free][arch-rsp] | [free][nix] | [free][cr-rsp] | [taken, no bin][npm-rsp] | [taken: Rapid SSH Proxy, installs `rsp`][py-rsp] ([setup.py][rsp-setup]) | oh-my-zsh rails: [`rsp='rails server --port'`][omz-rails-rsp] | [Snawoot/rsp][gh-rsp] 356 stars |
| `rspr` | [free][brew-api] | [free][deb-rspr] | [free][arch-rspr] | [free][nix] | [free][cr-rspr] | [free][npm-rspr] | [free][py-rspr] | free | [cwhidden/rspr][gh-rspr] 12 stars, a C++ tool that builds an `rspr` binary ([Makefile][rspr-make]) |
| `rns` | [free][brew-api] | [free][deb-rns] | [free][arch-rns] | [`rns` is the Reticulum Python package, no `rns` program][nix] | [taken, bin `rns`, network scanner, updated 2025-11-04][cr-rns] | [taken, no bin][npm-rns] | [taken: Reticulum Network Stack 1.5.7][py-rns] | oh-my-zsh react-native: [`rns='react-native start'`][omz-rn] | exact-name repos at most 34 stars ([search][gh-s-rns]) |
| `rnsp` | [free][brew-api] | [free][deb-rnsp] | [free][arch-rnsp] | [free][nix] | [free][cr-rnsp] | [free][npm-rnsp] | [free][py-rnsp] | free | exact-name repos have 0 or 1 star ([search][gh-s-rnsp]) |
| `rnspr` | [free][brew-api] | [free][deb-rnspr] | [free][arch-rnspr] | [free][nix] | [free][cr-rnspr] | [free][npm-rnspr] | [free][py-rnspr] | free | no exact-name repo ([search][gh-s-rnspr]) |
| `spr` | [formula `spr`, installs `spr`][brew-spr] | [free][deb-spr] | [free][arch-spr] | [`spr`, mainProgram `spr`][nix] | [taken, bin `spr`][cr-spr] | [taken, bin `spr`][npm-spr] | [taken][py-spr] | free | [spacedentist/spr][gh-spr] 539 stars (the Homebrew formula); [ejoffe/spr][gh-ejoffe] 1,291 stars ships `git-spr` ([.goreleaser.yml][ejoffe-gr]) |
| `spore` | [free][brew-api] | [free][deb-spore] | [free][arch-spore] | [free][nix] | [taken: 0.1.0 "Coming soon...", 2019, no bin][cr-spore] | [taken: ReST client from 2011, no bin][npm-spore] | [taken: P2P framework, last upload 2014; console scripts not checked][py-spore] | free | exact-name repos at most 86 stars, none a popular CLI: [wiremas/spore][gh-wiremas] (Maya plugin), [mhhf/spore][gh-mhhf] (JS package manager, last push 2015), [Pebaz/spore][gh-pebaz] 61 stars (Rust UEFI disassembler) |
| `spor` | [free][brew-api] | [free][deb-spor] | [free][arch-spor] | [free][nix] | [taken, bin `spor`, 2020][cr-spor] | [free][npm-spor] | [taken][py-spor] | free | no exact-name repo in top 50 by stars ([search][gh-s-spor]) |
| `spo` | [free][brew-api] | [free][deb-spo] | [free][arch-spo] | [free][nix] | [free][cr-spo] | [taken: 0.0.0, no bin][npm-spo] | [taken][py-spo] | free | no exact-name repo in top 50 by stars ([search][gh-s-spo]) |
| `sprun` | [free][brew-api] | [free][deb-sprun] | [free][arch-sprun] | [free][nix] | [free][cr-sprun] | [free][npm-sprun] | [taken: 0.0.3][py-sprun] | free | one exact-name repo, 1 star ([search][gh-s-sprun]) |
| `rsr` | [free][brew-api] | [free][deb-rsr] | [free][arch-rsr] | [free][nix] | [free][cr-rsr] | [taken, no bin][npm-rsr] | [taken][py-rsr] | free | exact-name repos at most 56 stars ([search][gh-s-rsr]) |
| `rspo` | [free][brew-api] | [free][deb-rspo] | [free][arch-rspo] | [free][nix] | [free][cr-rspo] | [free][npm-rspo] | [free][py-rspo] | free | exact-name repos at most 2 stars ([search][gh-s-rspo]) |

Notes on the table:

- No candidate is a zsh or bash built-in. `zsh -fc 'whence -w <name>'` and `bash --norc -c 'type -t <name>'` returned nothing for all except `rs`, which is `/usr/bin/rs` on macOS 27.0.
- Only `rs` is in the macOS base system (`/bin`, `/sbin`, `/usr/bin`, `/usr/sbin`, `/usr/libexec`).
- Debian was checked in stable (trixie) and unstable (sid); Ubuntu in noble. All three agree for every name except `sp`, where sid could not be parsed.
- Arch was checked by exact package name only. The Arch website has no file-ownership search, so an Arch package that ships a binary under another package name would be missed.
- Nix was checked against the search.nixos.org index for `nixos-unstable`, matching package name, attribute name, `mainProgram` and the `package_programs` list. A control query for `rg` correctly returned ripgrep.
- GitHub code search for `[[bin]] name = "<name>"` in `Cargo.toml` and `"<name>":` with `bin` in `package.json` is token-based, so it returns loose matches. For `spore` it found small Rust repos with a `spore` bin: [Pebaz/spore][gh-pebaz], [spore-lang/spore][gh-spore-lang] (0 stars) and [teddytennant/spore][gh-teddy] (2 stars). For `rsp` it matched [adobe/react-spectrum][gh-rs-codemods], but that bin is named `codemods`, not `rsp`.
- Homebrew matching used formula name, aliases, old names and the `executables` list (populated for 7,269 of 8,647 formulae), plus cask tokens and cask `binary` targets.

Discarded early: `rs` and `sp` fail at once (macOS base tool, Debian binary, several oh-my-zsh aliases). `rsp`, `rns` and `spr` each collide with a real command or a common alias. They were still checked in full so the reasons are on record.

## Is `runspore` itself free?

| Where | Result | Source |
|---|---|---|
| crates.io | free (HTTP 404) | [crates.io API][cr-runspore] |
| npm | free (HTTP 404) | [npm registry][npm-runspore] |
| PyPI | free (HTTP 404) | [PyPI JSON API][py-runspore] |
| GitHub user or org `runspore` | free (HTTP 404 on both) | [users API][gh-user], [orgs API][gh-org] |
| GitHub repositories | only `Web-tree/runspore`, the project's own repo | [repo search][gh-s-runspore] |
| Homebrew, Debian, Ubuntu, Arch, Nix, oh-my-zsh | free | same queries as the table |

"Free" on a registry means no package exists today. It does not reserve the name. Nothing was registered during this research.

## Reasoning

**Why `spore`.** It is five letters, says what the product is, and reads well: `spore run x.json`, `spore status`, `spore signal`. None of Homebrew, Debian, Ubuntu, Arch or Nix installs a `spore` executable, and no oh-my-zsh plugin aliases it (see the table). The registry names are taken, but by stale packages: a 2019 placeholder crate with no bin ([crates.io][cr-spore]), a 2011 npm package with no bin ([npm][npm-spore]), and a PyPI package last uploaded in 2014 ([PyPI][py-spore]; whether it installs a `spore` script was not checked). So publishing as `runspore` with a `spore` binary does not shadow any command found in the sources checked.

**Costs of `spore`.**

- The package name and the command differ: `cargo install runspore` and `npm i -g runspore` would install `spore`. Many tools do this, for example ripgrep installs `rg` ([Homebrew executables list][brew-rg]). The install docs must say so once.
- It is an ordinary English word. Several small GitHub projects already use it, and a few have a Rust `spore` bin (see notes). A future popular `spore` tool is more likely than for an abbreviation.
- `cargo install spore` would fetch the unrelated 2019 placeholder crate. Docs should always show `cargo install runspore`.

**Why `rnsp` is the runner-up.** It is one of three candidates (with `rnspr` and `rspo`) that have no occupant in any packaging source or registry: Homebrew, Debian, Ubuntu, Arch, Nix, crates.io, npm, PyPI and oh-my-zsh are all free, and GitHub has only 0 or 1 star repos with that name. At four letters it ties with `rspo` for shortest, and it is the more obvious contraction of runspore. But it is not a word, it is hard to say aloud, and its letters are easy to transpose (`rnps`, `rsnp`). `rspr` is free in every registry too, but it already names a C++ phylogenetics binary ([cwhidden/rspr][gh-rspr]).

**Trade-off against keeping `runspore`.** `runspore` is free everywhere checked and matches the package names, so there is zero confusion. It is eight letters, and tab completion on `runs` is ambiguous when runit or minicom is installed, because runit ships `runsv`, `runsvdir` and `runsvchdir` ([runit][brew-runit]) and minicom ships `runscript` ([minicom][brew-minicom]), per the Homebrew `executables` lists. Keeping it is a fine answer if the team prefers one name everywhere.

**Suggested approach.** Ship the binary as `spore`. Keep `runspore` as the crate, npm package, Homebrew formula and repo name. Optionally also install a `runspore` symlink, so people who know only the product name can still run it. Check `spore` again before the first Homebrew or Linux package release, since these sources change.

## Method

All checks ran on 2026-10-06 from macOS 27.0. Read-only: nothing was registered, reserved or published.

**Homebrew.** Downloaded both indexes, then matched each name against formula `name`, `aliases`, `oldnames`, `executables`, cask `token`, `old_tokens` and cask `binary` artifacts and their `target`.

```
curl -sS -o formula.json https://formulae.brew.sh/api/formula.json
curl -sS -o cask.json    https://formulae.brew.sh/api/cask.json
```

**Debian and Ubuntu.** Exact-filename contents search, for each `<name>`:

```
https://packages.debian.org/search?searchon=contents&keywords=<name>&mode=exactfilename&suite=stable&arch=any
https://packages.debian.org/search?searchon=contents&keywords=<name>&mode=exactfilename&suite=unstable&arch=any
https://packages.ubuntu.com/search?searchon=contents&keywords=<name>&mode=exactfilename&suite=noble&arch=any
```

**Arch Linux.** Exact package name, plus a keyword search to look for near matches:

```
https://archlinux.org/packages/search/json/?name=<name>
https://archlinux.org/packages/search/json/?q=<name>
```

**Nixpkgs.** The Elasticsearch backend used by search.nixos.org. The read-only credentials are the public ones in the site's frontend script (`/static/js/index.771b9a4a.js`). Index alias `latest-51-nixos-unstable`.

```
curl -s -u aWVSALXpZv:X8gPHnzL52wFEekuxsfQ9cSh -H 'Content-Type: application/json' \
  https://search.nixos.org/backend/latest-51-nixos-unstable/_search \
  -d '{"query":{"bool":{"filter":[{"term":{"type":"package"}}],"should":[
        {"term":{"package_programs":"<name>"}},{"term":{"package_pname":"<name>"}},
        {"term":{"package_attr_name":"<name>"}},{"term":{"package_mainProgram":"<name>"}}],
        "minimum_should_match":1}}}'
```

**Registries.** HTTP status 200 means taken, 404 means free. Details (description, version, bin names, dates) came from the same JSON.

```
curl -A "research-script/0.1 (read-only name availability check)" https://crates.io/api/v1/crates/<name>
curl https://registry.npmjs.org/<name>
curl https://pypi.org/pypi/<name>/json
curl https://api.npmjs.org/downloads/point/last-week/<name>
```

The crates.io response lists `bin_names` per version, which is where "bin `rns`", "no bin" and so on come from. npm "bin" comes from the `bin` field of the latest version.

**Shell aliases and built-ins.**

```
git clone --depth 1 https://github.com/ohmyzsh/ohmyzsh.git   # commit 60c9a7a839b790cd905d0fd4419435124fd1bdc0
grep -rnE "alias +(-g +)?['\"]?<name>['\"]?=" ohmyzsh
grep -rnE "^(function +)?<name> *\(\)" ohmyzsh
zsh -fc 'whence -w <name>'
bash --norc --noprofile -c 'type -t <name>'
zsh -ic 'whence -v <name>'
ls /bin /sbin /usr/bin /usr/sbin /usr/libexec | grep -x <name>
```

The grep pattern was checked against a known alias (`gst='git status'` in the git plugin). Prezto and other frameworks were not checked.

**GitHub** (authenticated `gh` CLI):

```
gh api -X GET search/repositories -f q="<name> in:name" -f sort=stars -f order=desc -f per_page=50
gh api -X GET search/code -f q='"name = \"<name>\"" "[[bin]]" filename:Cargo.toml'
gh api -X GET search/code -f q='"\"<name>\":" "bin" filename:package.json'
gh api repos/<owner>/<repo>
curl https://api.github.com/users/runspore
curl https://api.github.com/orgs/runspore
```

Repository search only looked at the top 50 results by stars. For very generic names (`rs`, `sp`, `spr`, `spor`, `spo`), an exact-name repo below that cut would be missed.

[brew-api]: https://formulae.brew.sh/api/formula.json
[brew-spr]: https://formulae.brew.sh/formula/spr
[brew-rg]: https://formulae.brew.sh/formula/ripgrep
[brew-runit]: https://formulae.brew.sh/formula/runit
[brew-minicom]: https://formulae.brew.sh/formula/minicom
[nix]: https://search.nixos.org/packages?channel=unstable

[deb-rs]: https://packages.debian.org/search?searchon=contents&keywords=rs&mode=exactfilename&suite=stable&arch=any
[deb-sp]: https://packages.debian.org/search?searchon=contents&keywords=sp&mode=exactfilename&suite=stable&arch=any
[deb-rsp]: https://packages.debian.org/search?searchon=contents&keywords=rsp&mode=exactfilename&suite=stable&arch=any
[deb-rspr]: https://packages.debian.org/search?searchon=contents&keywords=rspr&mode=exactfilename&suite=stable&arch=any
[deb-rns]: https://packages.debian.org/search?searchon=contents&keywords=rns&mode=exactfilename&suite=stable&arch=any
[deb-rnsp]: https://packages.debian.org/search?searchon=contents&keywords=rnsp&mode=exactfilename&suite=stable&arch=any
[deb-rnspr]: https://packages.debian.org/search?searchon=contents&keywords=rnspr&mode=exactfilename&suite=stable&arch=any
[deb-spr]: https://packages.debian.org/search?searchon=contents&keywords=spr&mode=exactfilename&suite=stable&arch=any
[deb-spore]: https://packages.debian.org/search?searchon=contents&keywords=spore&mode=exactfilename&suite=stable&arch=any
[deb-spor]: https://packages.debian.org/search?searchon=contents&keywords=spor&mode=exactfilename&suite=stable&arch=any
[deb-spo]: https://packages.debian.org/search?searchon=contents&keywords=spo&mode=exactfilename&suite=stable&arch=any
[deb-sprun]: https://packages.debian.org/search?searchon=contents&keywords=sprun&mode=exactfilename&suite=stable&arch=any
[deb-rsr]: https://packages.debian.org/search?searchon=contents&keywords=rsr&mode=exactfilename&suite=stable&arch=any
[deb-rspo]: https://packages.debian.org/search?searchon=contents&keywords=rspo&mode=exactfilename&suite=stable&arch=any

[arch-rs]: https://archlinux.org/packages/search/json/?name=rs
[arch-sp]: https://archlinux.org/packages/search/json/?name=sp
[arch-rsp]: https://archlinux.org/packages/search/json/?name=rsp
[arch-rspr]: https://archlinux.org/packages/search/json/?name=rspr
[arch-rns]: https://archlinux.org/packages/search/json/?name=rns
[arch-rnsp]: https://archlinux.org/packages/search/json/?name=rnsp
[arch-rnspr]: https://archlinux.org/packages/search/json/?name=rnspr
[arch-spr]: https://archlinux.org/packages/search/json/?name=spr
[arch-spore]: https://archlinux.org/packages/search/json/?name=spore
[arch-spor]: https://archlinux.org/packages/search/json/?name=spor
[arch-spo]: https://archlinux.org/packages/search/json/?name=spo
[arch-sprun]: https://archlinux.org/packages/search/json/?name=sprun
[arch-rsr]: https://archlinux.org/packages/search/json/?name=rsr
[arch-rspo]: https://archlinux.org/packages/search/json/?name=rspo

[cr-rs]: https://crates.io/api/v1/crates/rs
[cr-sp]: https://crates.io/api/v1/crates/sp
[cr-rsp]: https://crates.io/api/v1/crates/rsp
[cr-rspr]: https://crates.io/api/v1/crates/rspr
[cr-rns]: https://crates.io/api/v1/crates/rns
[cr-rnsp]: https://crates.io/api/v1/crates/rnsp
[cr-rnspr]: https://crates.io/api/v1/crates/rnspr
[cr-spr]: https://crates.io/api/v1/crates/spr
[cr-spore]: https://crates.io/api/v1/crates/spore
[cr-spor]: https://crates.io/api/v1/crates/spor
[cr-spo]: https://crates.io/api/v1/crates/spo
[cr-sprun]: https://crates.io/api/v1/crates/sprun
[cr-rsr]: https://crates.io/api/v1/crates/rsr
[cr-rspo]: https://crates.io/api/v1/crates/rspo
[cr-runspore]: https://crates.io/api/v1/crates/runspore

[npm-rs]: https://registry.npmjs.org/rs
[npm-sp]: https://registry.npmjs.org/sp
[npm-rsp]: https://registry.npmjs.org/rsp
[npm-rspr]: https://registry.npmjs.org/rspr
[npm-rns]: https://registry.npmjs.org/rns
[npm-rnsp]: https://registry.npmjs.org/rnsp
[npm-rnspr]: https://registry.npmjs.org/rnspr
[npm-spr]: https://registry.npmjs.org/spr
[npm-spore]: https://registry.npmjs.org/spore
[npm-spor]: https://registry.npmjs.org/spor
[npm-spo]: https://registry.npmjs.org/spo
[npm-sprun]: https://registry.npmjs.org/sprun
[npm-rsr]: https://registry.npmjs.org/rsr
[npm-rspo]: https://registry.npmjs.org/rspo
[npm-runspore]: https://registry.npmjs.org/runspore

[py-rs]: https://pypi.org/pypi/rs/json
[py-sp]: https://pypi.org/pypi/sp/json
[py-rsp]: https://pypi.org/pypi/rsp/json
[py-rspr]: https://pypi.org/pypi/rspr/json
[py-rns]: https://pypi.org/pypi/rns/json
[py-rnsp]: https://pypi.org/pypi/rnsp/json
[py-rnspr]: https://pypi.org/pypi/rnspr/json
[py-spr]: https://pypi.org/pypi/spr/json
[py-spore]: https://pypi.org/pypi/spore/json
[py-spor]: https://pypi.org/pypi/spor/json
[py-spo]: https://pypi.org/pypi/spo/json
[py-sprun]: https://pypi.org/pypi/sprun/json
[py-rsr]: https://pypi.org/pypi/rsr/json
[py-rspo]: https://pypi.org/pypi/rspo/json
[py-runspore]: https://pypi.org/pypi/runspore/json

[omz-repo]: https://github.com/ohmyzsh/ohmyzsh/blob/60c9a7a839b790cd905d0fd4419435124fd1bdc0/plugins/repo/repo.plugin.zsh#L3
[omz-rails-rs]: https://github.com/ohmyzsh/ohmyzsh/blob/60c9a7a839b790cd905d0fd4419435124fd1bdc0/plugins/rails/rails.plugin.zsh#L66
[omz-rails-rsp]: https://github.com/ohmyzsh/ohmyzsh/blob/60c9a7a839b790cd905d0fd4419435124fd1bdc0/plugins/rails/rails.plugin.zsh#L69
[omz-rails-sp]: https://github.com/ohmyzsh/ohmyzsh/blob/60c9a7a839b790cd905d0fd4419435124fd1bdc0/plugins/rails/rails.plugin.zsh#L144
[omz-ws]: https://github.com/ohmyzsh/ohmyzsh/blob/60c9a7a839b790cd905d0fd4419435124fd1bdc0/plugins/web-search/web-search.plugin.zsh#L80
[omz-sc]: https://github.com/ohmyzsh/ohmyzsh/blob/60c9a7a839b790cd905d0fd4419435124fd1bdc0/plugins/singlechar/singlechar.plugin.zsh#L89
[omz-rn]: https://github.com/ohmyzsh/ohmyzsh/blob/60c9a7a839b790cd905d0fd4419435124fd1bdc0/plugins/react-native/react-native.plugin.zsh#L3

[gh-rsp]: https://github.com/Snawoot/rsp
[rsp-setup]: https://github.com/Snawoot/rsp/blob/HEAD/setup.py
[gh-rspr]: https://github.com/cwhidden/rspr
[rspr-make]: https://github.com/cwhidden/rspr/blob/HEAD/Makefile
[gh-spr]: https://github.com/spacedentist/spr
[gh-ejoffe]: https://github.com/ejoffe/spr
[ejoffe-gr]: https://github.com/ejoffe/spr/blob/HEAD/.goreleaser.yml
[gh-wiremas]: https://github.com/wiremas/spore
[gh-mhhf]: https://github.com/mhhf/spore
[gh-pebaz]: https://github.com/Pebaz/spore
[gh-spore-lang]: https://github.com/spore-lang/spore
[gh-teddy]: https://github.com/teddytennant/spore
[gh-rs-codemods]: https://github.com/adobe/react-spectrum/blob/HEAD/packages/dev/codemods/package.json
[gh-user]: https://api.github.com/users/runspore
[gh-org]: https://api.github.com/orgs/runspore
[gh-s-rns]: https://github.com/search?q=rns+in%3Aname&type=repositories&s=stars&o=desc
[gh-s-rnsp]: https://github.com/search?q=rnsp+in%3Aname&type=repositories&s=stars&o=desc
[gh-s-rnspr]: https://github.com/search?q=rnspr+in%3Aname&type=repositories&s=stars&o=desc
[gh-s-spor]: https://github.com/search?q=spor+in%3Aname&type=repositories&s=stars&o=desc
[gh-s-spo]: https://github.com/search?q=spo+in%3Aname&type=repositories&s=stars&o=desc
[gh-s-sprun]: https://github.com/search?q=sprun+in%3Aname&type=repositories&s=stars&o=desc
[gh-s-rsr]: https://github.com/search?q=rsr+in%3Aname&type=repositories&s=stars&o=desc
[gh-s-rspo]: https://github.com/search?q=rspo+in%3Aname&type=repositories&s=stars&o=desc
[gh-s-runspore]: https://github.com/search?q=runspore+in%3Aname&type=repositories
