# Changelog

All notable changes to this project will be documented in this file. See [commit-and-tag-version](https://github.com/absolute-version/commit-and-tag-version) for commit guidelines.

## [0.8.3](https://github.com/blackopsrepl/Tranche/compare/v0.8.2...v0.8.3) (2026-10-02)


### Features

* **cli:** migrate Tranche to Rust ([#20](https://github.com/blackopsrepl/Tranche/issues/20)) ([bbfa017](https://github.com/blackopsrepl/Tranche/commit/bbfa0177446fc119abf33381626d1b631fb8e0e4))
* **cli:** serve the MCP surface as a subcommand ([bb39663](https://github.com/blackopsrepl/Tranche/commit/bb396639d6d7446bb7a412d9efb1caebdbc463bc))


### Bug Fixes

* **ci:** use a current Rust toolchain and publish Forgejo by release ID ([fdce351](https://github.com/blackopsrepl/Tranche/commit/fdce3514193ddfe863998192d4bfb0f3c6927967))
* **install:** target the CLI crate and document the native surfaces ([2707623](https://github.com/blackopsrepl/Tranche/commit/270762348eea170a244435bb25037b0323516897))
* **mcp:** honor the advertised tool argument contract ([117536b](https://github.com/blackopsrepl/Tranche/commit/117536bbcff10eb60c8ed166a24f1e4312d6e6c4))
* **mcp:** restore clean module boundaries after the split ([069f65d](https://github.com/blackopsrepl/Tranche/commit/069f65dc44cdbd0b8e740b0097d630a132d38582))
* **release:** bump the Rust versions with the release tag ([a854e7d](https://github.com/blackopsrepl/Tranche/commit/a854e7d29e98bffb284b74892291021297105ecf))

## [0.8.2](https://github.com/blackopsrepl/Tranche/compare/v0.8.1...v0.8.2) (2026-10-02)


### Features

* **evidence:** add a bounded GET-only GitHub read transport ([2e267fb](https://github.com/blackopsrepl/Tranche/commit/2e267fb9ca38f257f9947250569caf7ae704d8d7))
* **evidence:** capture, inspect and export a batch's native evidence ([f6e1116](https://github.com/blackopsrepl/Tranche/commit/f6e111637686c694757047b12042c0e53c95f4dc))


### Bug Fixes

* **evidence:** correct pagination, writer exclusion and revision checks ([f262be3](https://github.com/blackopsrepl/Tranche/commit/f262be3aeb0b1ada581fc5392192fca999b8aa29))

## [0.8.1](https://github.com/blackopsrepl/Tranche/compare/v0.8.0...v0.8.1) (2026-10-01)

### Bug Fixes

* **docs:** human-facing README with quick start, MCP maintenance guide and FAQ ([a24cb3e](https://github.com/blackopsrepl/Tranche/commit/a24cb3eca5e48a8892bb2ac8907e4ef1de14ce5b))

## [0.8.0](https://github.com/blackopsrepl/Tranche/compare/v0.7.0...v0.8.0) (2026-10-01)

### Features

* add a user-experience category reserved for DHH ([c8a1206](https://github.com/blackopsrepl/Tranche/commit/c8a12061504206850f9735b4d604aa97e2087f3d)), closes [#14](https://github.com/blackopsrepl/Tranche/issues/14)

### Bug Fixes

* retire the pre-binding compatibility fallback in resume reuse ([9195fc1](https://github.com/blackopsrepl/Tranche/commit/9195fc12f026c9e7b3f43fedf14decec18e08533)), references [#15](https://github.com/blackopsrepl/Tranche/issues/15)

## [0.7.0](https://github.com/blackopsrepl/Tranche/compare/v0.6.0...v0.7.0) (2026-10-01)

### Features

* **batches:** stale-first ordering from evidenced head activity ([d06bb17](https://github.com/blackopsrepl/Tranche/commit/d06bb17ea79e68e3be4567cc89eae44e1fb6a59e)), references [#11](https://github.com/blackopsrepl/Tranche/issues/11)

## [0.6.0](https://github.com/blackopsrepl/Tranche/compare/v0.5.0...v0.6.0) (2026-10-01)

### Features

* **page:** include parked coverage in the render summary ([e264d6c](https://github.com/blackopsrepl/Tranche/commit/e264d6c10217f4d843fd8ac2f84a2d1a8b156df3))
* **refresh:** report parked accounting in the refresh summary ([f10151a](https://github.com/blackopsrepl/Tranche/commit/f10151a37558f4f6ec293fc64bc97c3dc8acd054))

## [0.5.0](https://github.com/blackopsrepl/Tranche/compare/v0.4.0...v0.5.0) (2026-10-01)

### Features

* **batches:** park ineligible PRs before packing ([71fb86b](https://github.com/blackopsrepl/Tranche/commit/71fb86bbb5118e675042c3e892d09effd98d070d))
* **mcp:** serve parked state bound to the producer predicate ([0c60b51](https://github.com/blackopsrepl/Tranche/commit/0c60b5109b0a31301774e017b8a5480d6073d8a5))
* **workbench:** park as a first-class queue with reasons and unblock paths ([3b3351d](https://github.com/blackopsrepl/Tranche/commit/3b3351d9cc7019b706b8eb42f248b9553072aa68))

## [0.4.0](https://github.com/blackopsrepl/Tranche/compare/v0.3.4...v0.4.0) (2026-10-01)

### Bug Fixes

* **refresh:** bind judgments to PR evidence, not the churning envelope ([c57bb6c](https://github.com/blackopsrepl/Tranche/commit/c57bb6c431f67bf74383be22d9cb2da2612c097e))

## [0.3.4](https://github.com/blackopsrepl/Tranche/compare/v0.3.3...v0.3.4) (2026-10-01)


### Bug Fixes

* **refs:** read repository-qualified body references as pair candidates ([de8df19](https://github.com/blackopsrepl/Tranche/commit/de8df196b2120d1bb3988813e6e2dca77b63ae62))

## [0.3.3](https://github.com/blackopsrepl/Tranche/compare/v0.3.2...v0.3.3) (2026-10-01)


### Bug Fixes

* **mcp:** enforce each tool's advertised argument schema ([099c50c](https://github.com/blackopsrepl/Tranche/commit/099c50c10064842c7e5595a878d522d529905350))

## [0.3.2](https://github.com/blackopsrepl/Tranche/compare/v0.3.1...v0.3.2) (2026-10-01)

## [0.3.1](https://github.com/blackopsrepl/Tranche/compare/v0.3.0...v0.3.1) (2026-10-01)

## [0.3.0](https://github.com/blackopsrepl/Tranche/compare/v0.2.2...v0.3.0) (2026-10-01)


### ⚠ BREAKING CHANGES

* **mcp:** requirements-mcp.txt is removed and mcp_server.py no longer
imports mcp; install nothing to serve.

### Features

* **mcp:** serve standard MCP over stdio without the SDK ([ddc36ea](https://github.com/blackopsrepl/Tranche/commit/ddc36ea623b4a792ff5a3a0fed7413aa488eb945))

## [0.2.2](https://github.com/blackopsrepl/Tranche/compare/v0.2.1...v0.2.2) (2026-10-01)


### Features

* **mcp:** expose bound triage reports over read-only stdio ([cb64874](https://github.com/blackopsrepl/Tranche/commit/cb6487486765fe88312416c765c1cc96d0e0355c)), closes [#6](https://github.com/blackopsrepl/Tranche/issues/6)

## [0.2.1](https://github.com/blackopsrepl/Tranche/compare/v0.2.0...v0.2.1) (2026-09-30)


### Bug Fixes

* **workbench:** batch wording matches the five-per-tranche model ([18bab3b](https://github.com/blackopsrepl/Tranche/commit/18bab3be840c89d01055d325f6b2c3f31f3920b0))
* **workbench:** drop the redundant Batched queue ([739d373](https://github.com/blackopsrepl/Tranche/commit/739d37379f5376d0f43f44641e1f43ee75ed8d36))

## [0.2.0](https://github.com/blackopsrepl/Tranche/compare/v0.1.1...v0.2.0) (2026-09-30)


### ⚠ BREAKING CHANGES

* **batches:** out/batches.json format_version 3 (batch_size, per-batch
review_prompt; ids now B001-style).

### Features

* **batches:** batches of five with per-batch reviewer agent prompts ([9b6e7c0](https://github.com/blackopsrepl/Tranche/commit/9b6e7c0aec7d3d5fd7243b1cb7b729fbf0c0d0b6))


### Bug Fixes

* **workbench:** batch section in the inspector keeps its heading, button and flow on separate lines ([390330a](https://github.com/blackopsrepl/Tranche/commit/390330ad413b831b0f8af3719672515a53a3ca48))

## [0.1.1](https://github.com/blackopsrepl/Tranche/compare/v0.1.0...v0.1.1) (2026-09-30)


### Features

* **workbench:** per-batch browsing and deep inspection ([de33722](https://github.com/blackopsrepl/Tranche/commit/de337221c0a490017a6ac9e8c135e38bab1924c4))

## [0.1.0](https://github.com/blackopsrepl/Tranche/compare/v0.0.2...v0.1.0) (2026-09-30)


### ⚠ BREAKING CHANGES

* **batches:** out/batches.json format_version 2 replaces the
per-category cumulative tier format.

### Features

* **batches:** batches are Jev-determined merge groups ([98f53d1](https://github.com/blackopsrepl/Tranche/commit/98f53d1ea2fda9e37481af7b220f900dd315fddc)), closes [#4](https://github.com/blackopsrepl/Tranche/issues/4)
* **workbench:** security as a browsable meta-category ([615c518](https://github.com/blackopsrepl/Tranche/commit/615c518f0ea38d4acb6a77216a665ae1d7fe7a42)), closes [#3](https://github.com/blackopsrepl/Tranche/issues/3)


### Bug Fixes

* **workbench:** drop the security sort that reordered every PR ([9008d7e](https://github.com/blackopsrepl/Tranche/commit/9008d7ed95314b5fdcb5ba9267e725dd3060f648)), closes [#3](https://github.com/blackopsrepl/Tranche/issues/3)

## [0.0.2](https://github.com/blackopsrepl/Tranche/compare/v0.0.1...v0.0.2) (2026-09-30)


### Features

* **batches:** cumulative pre-release batches per category ([a2518b2](https://github.com/blackopsrepl/Tranche/commit/a2518b29b5703ffdfa7ca9bca7e35db35c66ca22)), closes [#4](https://github.com/blackopsrepl/Tranche/issues/4)
* **cluster:** security meta-category with top priority ([f7dc8e5](https://github.com/blackopsrepl/Tranche/commit/f7dc8e5f262acd7ae83311fdabb8608bea5bcafb)), closes [#3](https://github.com/blackopsrepl/Tranche/issues/3)
* **report:** restore OMARCHY branding with the Tranche keeper ([def48aa](https://github.com/blackopsrepl/Tranche/commit/def48aae20a72145e9ce9db87f16ee61b2eb2c47))
* **reports:** suggested pre-release batch plan in tranches.md ([d0c3fc3](https://github.com/blackopsrepl/Tranche/commit/d0c3fc354993afc64c075202d3b2d63e2d949afc)), closes [#4](https://github.com/blackopsrepl/Tranche/issues/4)
* **workbench:** browse pre-release batches per PR ([3f0bea3](https://github.com/blackopsrepl/Tranche/commit/3f0bea3231c1f2a4b0d0d130e226caf40d6a8719)), closes [#4](https://github.com/blackopsrepl/Tranche/issues/4)
* **workbench:** make the PR backlog searchable and browsable ([fa4022b](https://github.com/blackopsrepl/Tranche/commit/fa4022b87835d07f197385d608fbd95047334064))
* **workbench:** security-first queue and sort ([983c890](https://github.com/blackopsrepl/Tranche/commit/983c8908bb26d36eb8aa2c81178cf11164082277)), closes [#3](https://github.com/blackopsrepl/Tranche/issues/3)


### Bug Fixes

* **workbench:** batched queue counter uses the same membership predicate as the filter ([8815d7e](https://github.com/blackopsrepl/Tranche/commit/8815d7e95dc047376ee7290c304e1e4efeacb469))

## 0.0.1 (2026-09-30)


### Features

* **brand:** rename the project and CLI to Tranche ([adf83c8](https://github.com/blackopsrepl/Tranche/commit/adf83c84fbd123bf4442779cb10226b308e1685f))
* glyphfx title GIF as page header, square corners, spiffy Makefile ([23ec0ca](https://github.com/blackopsrepl/Tranche/commit/23ec0cac2693c64777a6f3e67b64141528fb1656))
* Jev-powered PR triage for the Omarchy triage team ([c5c159f](https://github.com/blackopsrepl/Tranche/commit/c5c159fdd418c74df8170f834b6bcfa62c5acf8c))
* OMARCHY block-glyph wordmark as GIF title, 'x Jev' tagline ([b16ca02](https://github.com/blackopsrepl/Tranche/commit/b16ca027707768febc1ab1b5c86415cd2a01067c))
* publishable GitHub Pages report generated from out/ data ([a9bbe02](https://github.com/blackopsrepl/Tranche/commit/a9bbe0262af5e1065eecacab355c723ecc70a037))
* **tranche:** land input-bound discovery and review prioritization ([3cfcca5](https://github.com/blackopsrepl/Tranche/commit/3cfcca56c22a039ff1e0f9ed0c22b7d55287fc1b))
* uncertain-pair tier + stale-ref guards in dupe pipeline ([e0c4a29](https://github.com/blackopsrepl/Tranche/commit/e0c4a2911502c71f958ecb5172253f671f55c3aa))


### Bug Fixes

* bind triage results to inputs and expose grouping uncertainty ([7b9a83b](https://github.com/blackopsrepl/Tranche/commit/7b9a83b08f25a6d59209212c698fe7acee77f5a0))
* extract PR cross-references from bodies as dupe candidates ([085c219](https://github.com/blackopsrepl/Tranche/commit/085c2190d10a20bf33b1ed3d9c25358a27a75a6b)), closes [#8590](https://github.com/blackopsrepl/Tranche/issues/8590) [#8771](https://github.com/blackopsrepl/Tranche/issues/8771) [#9966](https://github.com/blackopsrepl/Tranche/issues/9966)
* **triage:** expose contradictory pair evidence consistently ([bb3fe8f](https://github.com/blackopsrepl/Tranche/commit/bb3fe8fda8eeac4ebffdea64a8e8f5ad77fdd949))
* **triage:** normalize model responses and recover poisoned caches ([8e7fdd0](https://github.com/blackopsrepl/Tranche/commit/8e7fdd009f60c6043a15fc0629de315e36e7ca47))
* use the official OMARCHY wordmark (brand SVG) for the title GIF ([3a88521](https://github.com/blackopsrepl/Tranche/commit/3a88521566d8f23086fc27ae0f38dc2b4477a835))
* **validation:** reject enormous integers before float conversion ([d057f27](https://github.com/blackopsrepl/Tranche/commit/d057f27fad916f83f9d2c00698ea2fc9393859b3))
