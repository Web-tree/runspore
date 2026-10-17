# Changelog

## [0.6.1](https://github.com/Web-tree/runspore/compare/v0.6.0...v0.6.1) (2026-10-17)


### Bug Fixes

* **deps:** update rust crate jsonschema to 0.59.0 ([#229](https://github.com/Web-tree/runspore/issues/229)) ([722f757](https://github.com/Web-tree/runspore/commit/722f757a209364d2d97299a0ffa9720dfafcfec3))
* **FG-237:** schema messages and the recorded writer use spore --version's version ([#238](https://github.com/Web-tree/runspore/issues/238)) ([be19c9d](https://github.com/Web-tree/runspore/commit/be19c9d8c78f6e8d60a98f2642f2e1cf80ff5660)), closes [#237](https://github.com/Web-tree/runspore/issues/237)

## [0.6.0](https://github.com/Web-tree/runspore/compare/v0.5.0...v0.6.0) (2026-10-17)


### Features

* **FG-223:** a build that is not a release names its commit and date ([#230](https://github.com/Web-tree/runspore/issues/230)) ([f0ccc68](https://github.com/Web-tree/runspore/commit/f0ccc6807906c068ba28b9ab2580eeb633e75dc6)), closes [#223](https://github.com/Web-tree/runspore/issues/223)
* **FG-224:** checkpoint G feedback: `spore status`, `list` and the session-start line tell a try whose pro… ([#231](https://github.com/Web-tree/runspore/issues/231)) ([fb6425e](https://github.com/Web-tree/runspore/commit/fb6425ed1d12b6e4b1249762f956785a7c3f26cd))


### Bug Fixes

* **FG-222:** checkpoint G feedback: A newer spore does not silently upgrade a database that older workers … ([#236](https://github.com/Web-tree/runspore/issues/236)) ([0e6e8c6](https://github.com/Web-tree/runspore/commit/0e6e8c66a14c0dbb9e8e21064d3015d9f837a1f2))

## [0.5.0](https://github.com/Web-tree/runspore/compare/v0.4.0...v0.5.0) (2026-10-16)


### Features

* **191:** keep the run database in ~/.spore, shared by every worktree of a repository ([#227](https://github.com/Web-tree/runspore/issues/227)) ([fcdb1f4](https://github.com/Web-tree/runspore/commit/fcdb1f4fa5adcbb0356f685706e53c0e7a3720bd)), closes [#191](https://github.com/Web-tree/runspore/issues/191)

## [0.4.0](https://github.com/Web-tree/runspore/compare/v0.3.0...v0.4.0) (2026-10-16)


### Features

* **build:** validate checks argv $get paths against what the step gives ([#211](https://github.com/Web-tree/runspore/issues/211)) ([1c86325](https://github.com/Web-tree/runspore/commit/1c863250a408a4cc488360f75e65e7f5b6db030e)), closes [#200](https://github.com/Web-tree/runspore/issues/200)
* **cli:** spore continue names the worker that holds the run's step ([#206](https://github.com/Web-tree/runspore/issues/206)) ([258ea01](https://github.com/Web-tree/runspore/commit/258ea0169ffcd4410b4de67bec29494bb2357c66)), closes [#199](https://github.com/Web-tree/runspore/issues/199)
* **FE-202:** a child run's steps get RUNSPORE_PARENT_RUN_ID ([#210](https://github.com/Web-tree/runspore/issues/210)) ([ab5f3d7](https://github.com/Web-tree/runspore/commit/ab5f3d7d536646ecbebad0aeee938ff9ac6d8653))
* **FF-217:** checkpoint F feedback: A worker takes the step that has waited longest, and a run that is fur… ([#220](https://github.com/Web-tree/runspore/issues/220)) ([a65d8d1](https://github.com/Web-tree/runspore/commit/a65d8d14e7fb149585a7c2fb32a7a280c2b97016))
* **host:** a worker on a full disk pauses instead of counting the try ([#198](https://github.com/Web-tree/runspore/issues/198)) ([#209](https://github.com/Web-tree/runspore/issues/209)) ([401a3f6](https://github.com/Web-tree/runspore/commit/401a3f6d61e6a7544982bb30a23e6449684b8c55))
* **R27:** the reference coding loop killed at every row of its table ([#193](https://github.com/Web-tree/runspore/issues/193)) ([6092199](https://github.com/Web-tree/runspore/commit/609219987a799ce8e658363e95f66b972630c141))
* **R28:** reconcile docs and glossary with 0.2 ([#215](https://github.com/Web-tree/runspore/issues/215)) ([5ea8821](https://github.com/Web-tree/runspore/commit/5ea88211fd475513fd1eca689e294da7850acbed))


### Bug Fixes

* **build:** a repeatable action with no retry.maxAttempts gets three tries ([#212](https://github.com/Web-tree/runspore/issues/212)) ([aa610d8](https://github.com/Web-tree/runspore/commit/aa610d8c3e0dc7b6a5bb2f740cb81630ec244d93))
* **cli:** a quarantined run shows quarantined as its status, with the reason and the release ([#207](https://github.com/Web-tree/runspore/issues/207)) ([c070a3c](https://github.com/Web-tree/runspore/commit/c070a3cd85d99d06888b004e63c7b74937ac6f47)), closes [#197](https://github.com/Web-tree/runspore/issues/197)
* **host:** continue, run and worker --run work the runs a run started ([#219](https://github.com/Web-tree/runspore/issues/219)) ([2c35814](https://github.com/Web-tree/runspore/commit/2c3581455d08453f695ddfb27b59151f06a5b319)), closes [#218](https://github.com/Web-tree/runspore/issues/218)
* **host:** result mode defaults a missing output to {} ([#208](https://github.com/Web-tree/runspore/issues/208)) ([8fcb443](https://github.com/Web-tree/runspore/commit/8fcb44345cd7162902d17a9a9763cb6ec8527f8b)), closes [#201](https://github.com/Web-tree/runspore/issues/201)
* **host:** run, continue and worker --run work only their own run ([#214](https://github.com/Web-tree/runspore/issues/214)) ([902b31a](https://github.com/Web-tree/runspore/commit/902b31ad7aaf94460202dc5242c7cee124828d6d)), closes [#204](https://github.com/Web-tree/runspore/issues/204)
* **R23:** spore continue --wait --json exits 0 when it printed the run's status ([#213](https://github.com/Web-tree/runspore/issues/213)) ([9849863](https://github.com/Web-tree/runspore/commit/984986316f7a5c5a60a94194c13c8daef39fff53))

## [0.3.0](https://github.com/Web-tree/runspore/compare/v0.2.0...v0.3.0) (2026-10-15)


### Features

* **cli:** events show each event's time, node, outcome and first line ([#171](https://github.com/Web-tree/runspore/issues/171)) ([867cde9](https://github.com/Web-tree/runspore/commit/867cde9004f7e9d55fc87272d5dfbe05b362e83f)), closes [#161](https://github.com/Web-tree/runspore/issues/161)
* **FC-160:** checkpoint C feedback: `spore status` shows a step as `scheduled` both while its program runs… ([#170](https://github.com/Web-tree/runspore/issues/170)) ([7ef95cd](https://github.com/Web-tree/runspore/commit/7ef95cd0e600e1859c8aa8a0a662b0d7d149df5f)), closes [#160](https://github.com/Web-tree/runspore/issues/160)
* **FC-162:** checkpoint C feedback: The features from checkpoint C feedback are documented only in spec/cl… ([#172](https://github.com/Web-tree/runspore/issues/172)) ([b55a0c8](https://github.com/Web-tree/runspore/commit/b55a0c867344ea6013c6daaf5a21c5c179b469b9))
* **R20:** handshake: spore capabilities and capability-missing ([#175](https://github.com/Web-tree/runspore/issues/175)) ([d0a0654](https://github.com/Web-tree/runspore/commit/d0a0654015171523a4ed73915df2ef83425a67fc)), closes [#70](https://github.com/Web-tree/runspore/issues/70)
* **R21:** async commands: start, holder handle, probe and watch ([#174](https://github.com/Web-tree/runspore/issues/174)) ([77b03be](https://github.com/Web-tree/runspore/commit/77b03bea0dd3246611e3cd07c9a7093b721a9696))
* **R22:** spore continue, list --unfinished and step show ([#173](https://github.com/Web-tree/runspore/issues/173)) ([5b0d262](https://github.com/Web-tree/runspore/commit/5b0d2625de02b34eac0a61613b4905533be3e579))


### Bug Fixes

* **cli:** a start key, signal ID or resolve ID used again names its run and the ways on ([#166](https://github.com/Web-tree/runspore/issues/166)) ([efadf25](https://github.com/Web-tree/runspore/commit/efadf2502d45e1164c1d603d7e65299e6b8fe768)), closes [#158](https://github.com/Web-tree/runspore/issues/158)
* **cli:** spore signal refuses an outcome the waiting node does not route ([#169](https://github.com/Web-tree/runspore/issues/169)) ([7ad85e8](https://github.com/Web-tree/runspore/commit/7ad85e82d41f2e43a2d757b7997e5427afc73113))
* **deps:** update rust crate wit-bindgen to 0.63.0 ([#152](https://github.com/Web-tree/runspore/issues/152)) ([d239409](https://github.com/Web-tree/runspore/commit/d2394099b3e4c46f814591782f3baed0d5646b16))
* **deps:** update the wasm-tools crates to 0.262.0 together ([#150](https://github.com/Web-tree/runspore/issues/150)) ([1256a16](https://github.com/Web-tree/runspore/commit/1256a168f9d00f10893733b1483064e28e3f59e2))
* **release:** drop the one-time release-as so the next release is 0.2.1 ([#167](https://github.com/Web-tree/runspore/issues/167)) ([77069a1](https://github.com/Web-tree/runspore/commit/77069a19be3f2a02f071f762b91e34014a99d42e))
* **release:** pin the release key ([#165](https://github.com/Web-tree/runspore/issues/165)) ([1f9aa81](https://github.com/Web-tree/runspore/commit/1f9aa814f5145014959e1cc89b58c111b0e1f380))

## [0.2.0](https://github.com/Web-tree/runspore/compare/v0.2.0...v0.2.0) (2026-10-15)


### Features

* **cli:** events show each event's time, node, outcome and first line ([#171](https://github.com/Web-tree/runspore/issues/171)) ([867cde9](https://github.com/Web-tree/runspore/commit/867cde9004f7e9d55fc87272d5dfbe05b362e83f)), closes [#161](https://github.com/Web-tree/runspore/issues/161)
* **FC-160:** checkpoint C feedback: `spore status` shows a step as `scheduled` both while its program runs… ([#170](https://github.com/Web-tree/runspore/issues/170)) ([7ef95cd](https://github.com/Web-tree/runspore/commit/7ef95cd0e600e1859c8aa8a0a662b0d7d149df5f)), closes [#160](https://github.com/Web-tree/runspore/issues/160)
* **FC-162:** checkpoint C feedback: The features from checkpoint C feedback are documented only in spec/cl… ([#172](https://github.com/Web-tree/runspore/issues/172)) ([b55a0c8](https://github.com/Web-tree/runspore/commit/b55a0c867344ea6013c6daaf5a21c5c179b469b9))
* **R20:** handshake: spore capabilities and capability-missing ([#175](https://github.com/Web-tree/runspore/issues/175)) ([d0a0654](https://github.com/Web-tree/runspore/commit/d0a0654015171523a4ed73915df2ef83425a67fc)), closes [#70](https://github.com/Web-tree/runspore/issues/70)
* **R21:** async commands: start, holder handle, probe and watch ([#174](https://github.com/Web-tree/runspore/issues/174)) ([77b03be](https://github.com/Web-tree/runspore/commit/77b03bea0dd3246611e3cd07c9a7093b721a9696))
* **R22:** spore continue, list --unfinished and step show ([#173](https://github.com/Web-tree/runspore/issues/173)) ([5b0d262](https://github.com/Web-tree/runspore/commit/5b0d2625de02b34eac0a61613b4905533be3e579))


### Bug Fixes

* **cli:** a start key, signal ID or resolve ID used again names its run and the ways on ([#166](https://github.com/Web-tree/runspore/issues/166)) ([efadf25](https://github.com/Web-tree/runspore/commit/efadf2502d45e1164c1d603d7e65299e6b8fe768)), closes [#158](https://github.com/Web-tree/runspore/issues/158)
* **cli:** spore signal refuses an outcome the waiting node does not route ([#169](https://github.com/Web-tree/runspore/issues/169)) ([7ad85e8](https://github.com/Web-tree/runspore/commit/7ad85e82d41f2e43a2d757b7997e5427afc73113))
* **deps:** update rust crate wit-bindgen to 0.63.0 ([#152](https://github.com/Web-tree/runspore/issues/152)) ([d239409](https://github.com/Web-tree/runspore/commit/d2394099b3e4c46f814591782f3baed0d5646b16))
* **deps:** update the wasm-tools crates to 0.262.0 together ([#150](https://github.com/Web-tree/runspore/issues/150)) ([1256a16](https://github.com/Web-tree/runspore/commit/1256a168f9d00f10893733b1483064e28e3f59e2))
* **release:** pin the release key ([#165](https://github.com/Web-tree/runspore/issues/165)) ([1f9aa81](https://github.com/Web-tree/runspore/commit/1f9aa814f5145014959e1cc89b58c111b0e1f380))

## [0.2.0](https://github.com/Web-tree/runspore/compare/v0.2.0...v0.2.0) (2026-10-14)


### ⚠ BREAKING CHANGES

* **cli:** rename the binary to spore

### Features

* **C1:** node params and mapping operators: $if, $uuid, $json, $join, $resultSchema ([#134](https://github.com/Web-tree/runspore/issues/134)) ([5833cbd](https://github.com/Web-tree/runspore/commit/5833cbd6e8b3244844436350defe03137afa7143))
* **C2:** command argv from mappings, with step.resumed for first try versus resume ([#136](https://github.com/Web-tree/runspore/issues/136)) ([98b01f4](https://github.com/Web-tree/runspore/commit/98b01f43575665a7d3d0647f1adb2d6801652b7a))
* **C3:** command result block: outcome and output mapped from JSON stdout ([#135](https://github.com/Web-tree/runspore/issues/135)) ([c3d6cc1](https://github.com/Web-tree/runspore/commit/c3d6cc182670d1ad7a843f3f9444fff44173a19f)), closes [#125](https://github.com/Web-tree/runspore/issues/125)
* **C4:** coding-loop as two files: a claude -p action defined once, reused with params; guide page for checkpoint C ([#137](https://github.com/Web-tree/runspore/issues/137)) ([6f6dc04](https://github.com/Web-tree/runspore/commit/6f6dc04ade8e59104ace227f6b00033666abffc2)), closes [#126](https://github.com/Web-tree/runspore/issues/126)
* **cli:** examples hello, review-loop, unsafe-deploy ([206e832](https://github.com/Web-tree/runspore/commit/206e8323212db1ad4cb3c9a9480855a3a683ea09))
* **cli:** rename the binary to spore ([d699c47](https://github.com/Web-tree/runspore/commit/d699c4772ca94d25f54373b1753a024e0e1b722b))
* **cli:** run and worker print each run change on stderr ([#156](https://github.com/Web-tree/runspore/issues/156)) ([485a677](https://github.com/Web-tree/runspore/commit/485a6778b2cb654c666cc51e71e2470af45f6bd9)), closes [#148](https://github.com/Web-tree/runspore/issues/148)
* **cli:** runs started from a step record their parent; list and status show the tree ([#155](https://github.com/Web-tree/runspore/issues/155)) ([7dede83](https://github.com/Web-tree/runspore/commit/7dede83c9e5660c1f2de55ebdb7ab8ec02922afa)), closes [#146](https://github.com/Web-tree/runspore/issues/146)
* **cli:** runspore binary with every command of spec/cli.md ([8818ad5](https://github.com/Web-tree/runspore/commit/8818ad5cbad49f66bdc72259f4dbd1bf2674a34e))
* **cli:** status says what a run waits for and how to move it on ([#154](https://github.com/Web-tree/runspore/issues/154)) ([e63b3b0](https://github.com/Web-tree/runspore/commit/e63b3b07f660e0cff6add3d34e05617a6a3c4805)), closes [#145](https://github.com/Web-tree/runspore/issues/145)
* **component:** kernel guest exporting world machine ([4be1bf7](https://github.com/Web-tree/runspore/commit/4be1bf707fabd59bd4cd18437b94acdf90ce2947))
* **D1:** a newer spore continues runs started by an older one (engine upgrade keeps the database) ([#130](https://github.com/Web-tree/runspore/issues/130)) ([2eef667](https://github.com/Web-tree/runspore/commit/2eef667aee122aa13b2a713b49340048a1624efe))
* **D2:** run the dogfood build with plain spore commands, no wrapper scripts ([#133](https://github.com/Web-tree/runspore/issues/133)) ([1cd0d74](https://github.com/Web-tree/runspore/commit/1cd0d74c9b56810612580e2d2c4924bef2e9e2a8))
* **FC-147:** onIntervention runs a workflow action when a run needs intervention or fails ([#153](https://github.com/Web-tree/runspore/issues/153)) ([ef1dafa](https://github.com/Web-tree/runspore/commit/ef1dafa774b6cff077288e279864b0f28a3326b9)), closes [#147](https://github.com/Web-tree/runspore/issues/147)
* freeze MVP contract (spec, runspore-types, kernel stub) ([7f42cf8](https://github.com/Web-tree/runspore/commit/7f42cf8b24a82492cdff0f12d9e43553f2eec664))
* **host:** a failed command keeps the end of stdout and stderr in its error ([#142](https://github.com/Web-tree/runspore/issues/142)) ([1c94264](https://github.com/Web-tree/runspore/commit/1c94264957a6cc2046d9fb642dee3b208f72a5c7)), closes [#140](https://github.com/Web-tree/runspore/issues/140)
* **host:** count diverged commits, configurable shutdown grace ([ffa4c11](https://github.com/Web-tree/runspore/commit/ffa4c11e32e9c33b6e4d0067b1dd1b29e916980b))
* **host:** engine, activity runners and command runner ([81bc357](https://github.com/Web-tree/runspore/commit/81bc3575c40e5e48d89a2ee13ecf2747e062823b))
* **host:** Engine::shutdown, used by serve, applies drained results ([5df3fbf](https://github.com/Web-tree/runspore/commit/5df3fbfbc2903190ddb38a2940218c7a253879be))
* **host:** random run IDs; a start key names a run within one database ([#83](https://github.com/Web-tree/runspore/issues/83)) ([8c789c0](https://github.com/Web-tree/runspore/commit/8c789c02675d9b0aae703b7bbcab25b670ae04d6))
* **host:** validate_workflow, the checks of start without a store ([c3d1ac7](https://github.com/Web-tree/runspore/commit/c3d1ac7de28ac0e75e4c38f0e6940ee8219f418a))
* **kernel:** accept package format 0.2 and semantics 0.2 beside 0.1 ([#80](https://github.com/Web-tree/runspore/issues/80)) ([571f08a](https://github.com/Web-tree/runspore/commit/571f08a6634eb11ef04902bb4e2fbe271dae8d87))
* **kernel:** deterministic graph kernel ([5edd305](https://github.com/Web-tree/runspore/commit/5edd305b0899ea600c2c2cb5e75628df6fe24f73))
* runspore_build::compose(kernel, artifact) encodes the composition ([d54814d](https://github.com/Web-tree/runspore/commit/d54814d1103145e4c024df1f7ee9a97250e513f4))
* **store:** SQLite adapter for the store protocol ([e0d7d05](https://github.com/Web-tree/runspore/commit/e0d7d05be51e544dac90330fb0a8ff5fe8cc8bc5))
* **trace:** runspore.trace/0.1 format, runner, compiler and generator ([ff710c2](https://github.com/Web-tree/runspore/commit/ff710c2ddc89ddd2e138845f57d66c5c84dffddb))
* **wasmtime:** build pipeline and WasmtimeReducer ([482d797](https://github.com/Web-tree/runspore/commit/482d797f37a5de65a35f38f7304d31fa085eab83))
* **wasmtime:** emit-component and overhead examples, README ([b8d9215](https://github.com/Web-tree/runspore/commit/b8d9215178492c275157360fb0cc63ac1d0430b4))


### Bug Fixes

* **build:** $resultSchema is an object at the top level, not anyOf ([#141](https://github.com/Web-tree/runspore/issues/141)) ([8e462e8](https://github.com/Web-tree/runspore/commit/8e462e8ee2f86ce3b0217a57a8e6dacd22146d3a)), closes [#139](https://github.com/Web-tree/runspore/issues/139)
* **cli:** crash suite counts re-runs of accepted steps from the crash on ([50885f8](https://github.com/Web-tree/runspore/commit/50885f866e0f156429c37fd2acc288c08f1f908f))
* **cli:** interrupts shut down through Engine::shutdown and exit 130 ([6b93320](https://github.com/Web-tree/runspore/commit/6b93320cf74cd4f8af5db091875641998464b056))
* **dogfood:** keep the build's database out of its steps' environment ([#92](https://github.com/Web-tree/runspore/issues/92)) ([2ef8ffa](https://github.com/Web-tree/runspore/commit/2ef8ffa4e7c189efa35b7c8d267b70cae8a1e4b8))
* **host:** abandon an attempt whose run or package cannot be read ([3315b53](https://github.com/Web-tree/runspore/commit/3315b53c21db174538af8a9e7f8cfce63454c3dd))
* **host:** engine nonce unique per engine, not per clock tick ([c11e4e1](https://github.com/Web-tree/runspore/commit/c11e4e10393c87218ba1ec333895ce8eb1e6be86))
* **host:** finish attempts stopped by shutdown instead of abandoning them ([191d2ef](https://github.com/Web-tree/runspore/commit/191d2ef07c1b98a81b9450f7a5af6e71704786a7))
* **kernel:** a waiting run takes its signal directly, so a full buffer cannot starve it ([03fa6c8](https://github.com/Web-tree/runspore/commit/03fa6c8acda55252f36a14711e55c3ae6c66619e))
