# Changelog

## [0.6.0](https://github.com/vaam-apps/vpay/compare/v0.5.0...v0.6.0) (2026-09-25)


### ⚠ BREAKING CHANGES

* vpay-sdk (sdks/rust) — ListPaymentIntentsParams, ListCheckoutSessionsParams and ListRefundsParams each gained a public `customer: Option<String>` field (in 8d328d4). Code that builds any of the three with a struct literal naming every field and no `..Default::default()` no longer compiles (E0063, missing field `customer`); add `customer: None` or `..Default::default()`. Code using Default::default() or field assignment, and every @vaam-apps/vpay-sdk caller, is unaffected.

### Features

* **checkout:** write a session's customer onto a customer-less intent (ADR-0025) ([#253](https://github.com/vaam-apps/vpay/issues/253)) ([fec2fc2](https://github.com/vaam-apps/vpay/commit/fec2fc230b28867b064c964003c0d48e4f451c80))
* customer filters on list endpoints and manual invoice payments (RFC-0004 step A) ([#251](https://github.com/vaam-apps/vpay/issues/251)) ([b747e5d](https://github.com/vaam-apps/vpay/commit/b747e5d5068c4fda6b6b1b9aa06041ddc32230d3))
* **dashboard:** move the dashboard to @vaam-apps/ui 0.4.0 ([#258](https://github.com/vaam-apps/vpay/issues/258)) ([f68fda0](https://github.com/vaam-apps/vpay/commit/f68fda097c66299fda183686288cadb6c8198955))


### Bug Fixes

* **dashboard:** the timeline note names only the two unwritten event types ([#247](https://github.com/vaam-apps/vpay/issues/247)) ([5129f60](https://github.com/vaam-apps/vpay/commit/5129f60c32701f130ce798ec7e1f1c21c2cfef7f))
* **erasure:** reach a payer's payments through their checkout sessions too (ADR-0027) ([#257](https://github.com/vaam-apps/vpay/issues/257)) ([1bba541](https://github.com/vaam-apps/vpay/commit/1bba54131e9d605052cf0f3892dbc9230cd49df9))
* **justfile:** make `just migrations-manifest` run on macOS ([9184e42](https://github.com/vaam-apps/vpay/commit/9184e42e68eb44c70ad27e5b9942d2a3378ca2f8))
* **justfile:** make just migrations-manifest run on macOS ([#252](https://github.com/vaam-apps/vpay/issues/252)) ([9184e42](https://github.com/vaam-apps/vpay/commit/9184e42e68eb44c70ad27e5b9942d2a3378ca2f8))
* **worker:** schedule every job on the database's clock, never the app host's (ADR-0026) ([#256](https://github.com/vaam-apps/vpay/issues/256)) ([434dda7](https://github.com/vaam-apps/vpay/commit/434dda7cc00d2e1a109c83e9b9fb10d6b2088366))


### Documentation

* **adr:** ADR-0024 — customer filters and manual payments (RFC-0004 step A) ([#248](https://github.com/vaam-apps/vpay/issues/248)) ([51c3c38](https://github.com/vaam-apps/vpay/commit/51c3c38dc852b8368c9805c3d9dee24189f09cc5))
* retire thirteen stale claims found re-verifying the skills, and make the route probe self-checking ([#255](https://github.com/vaam-apps/vpay/issues/255)) ([1d20640](https://github.com/vaam-apps/vpay/commit/1d20640677c13218136bf5f52cab6faba64d5e03))


### Tests

* read "now" off the database in fixtures that Postgres' now() judges ([#254](https://github.com/vaam-apps/vpay/issues/254)) ([d08dafd](https://github.com/vaam-apps/vpay/commit/d08dafdd1e833c23cbaad7fe95b9613738925a86))


### Chores

* **deps:** bump @vaam-apps/ui to v0.2.4 ([#249](https://github.com/vaam-apps/vpay/issues/249)) ([c95e257](https://github.com/vaam-apps/vpay/commit/c95e2573d463b058b7768caa2848bf8231e7f121))

## [0.5.0](https://github.com/vaam-apps/vpay/compare/v0.4.1...v0.5.0) (2026-09-23)


### Features

* **sdks:** Tauri v2 checkout plugin for Android, iOS and web ([#238](https://github.com/vaam-apps/vpay/issues/238)) ([999a23f](https://github.com/vaam-apps/vpay/commit/999a23f95e196f56d76d635156aa3d50325a7288))


### Bug Fixes

* **examples:** repair the tauri-checkout example's clean-checkout build ([#241](https://github.com/vaam-apps/vpay/issues/241)) ([dd1a48b](https://github.com/vaam-apps/vpay/commit/dd1a48b005c9e0e93a681ec7a491bdf53031d2ab))
* **sdks:** align the Tauri plugin's version with master's 0.4.1 ([#240](https://github.com/vaam-apps/vpay/issues/240)) ([078fa3d](https://github.com/vaam-apps/vpay/commit/078fa3d88f15f7cb5291bdbd8ad561d4a9a4ed4c))


### Documentation

* link the human documentation site, vpay-oss.vaam.store ([#245](https://github.com/vaam-apps/vpay/issues/245)) ([7f1b108](https://github.com/vaam-apps/vpay/commit/7f1b1081097cc56a24ad663d02fccf7c075f28a0))
* retire stale claims found while writing vpay-docs ([#243](https://github.com/vaam-apps/vpay/issues/243)) ([ac6e78c](https://github.com/vaam-apps/vpay/commit/ac6e78c9792cba235f68ec5083a6cefe5eecd016))
* **rfc:** propose billing, payment routing, direct card processing and bank reconciliation ([#244](https://github.com/vaam-apps/vpay/issues/244)) ([7997536](https://github.com/vaam-apps/vpay/commit/7997536b7efddc85ce994047e6e7ca02fe451368))
* **tauri:** correct the claims [#240](https://github.com/vaam-apps/vpay/issues/240) and [#241](https://github.com/vaam-apps/vpay/issues/241) left stale, and write down two silent gaps ([#242](https://github.com/vaam-apps/vpay/issues/242)) ([51082a7](https://github.com/vaam-apps/vpay/commit/51082a7ab1cbcd437e7a200dcada6c358631c58a))


### Continuous Integration

* tell vpay-docs when a release lands ([#246](https://github.com/vaam-apps/vpay/issues/246)) ([26b0bb6](https://github.com/vaam-apps/vpay/commit/26b0bb6557a1ce86ab5909242a490dc964a81afe))


### Chores

* **ci:** bump install-cratestack-cli to pick up the download retry fix ([#237](https://github.com/vaam-apps/vpay/issues/237)) ([e0889e5](https://github.com/vaam-apps/vpay/commit/e0889e51fbe586ebb4f5eb7b8b30c0150efe3359))

## [0.4.1](https://github.com/vaam-apps/vpay/compare/v0.4.0...v0.4.1) (2026-09-21)


### Chores

* **ci:** bump the vaam-apps/.github workflow pin to pick up the lint fix ([#235](https://github.com/vaam-apps/vpay/issues/235)) ([7134ecb](https://github.com/vaam-apps/vpay/commit/7134ecbbaa7466f78216dd94571fc0bd045ff9dd))

## [0.4.0](https://github.com/vaam-apps/vpay/compare/v0.3.1...v0.4.0) (2026-09-20)


### Features

* **xtask:** verify-doc-counts, so a number in a document cannot drift silently ([#233](https://github.com/vaam-apps/vpay/issues/233)) ([67c90ea](https://github.com/vaam-apps/vpay/commit/67c90ea5335b5baa032783d68ac4c80198dd89ac))

## [0.3.1](https://github.com/vaam-apps/vpay/compare/v0.3.0...v0.3.1) (2026-09-20)


### Bug Fixes

* **release:** publish-chart signs, refuses a stale version, and can resume ([#226](https://github.com/vaam-apps/vpay/issues/226)) ([8092c3d](https://github.com/vaam-apps/vpay/commit/8092c3d8f06acc21b2146d673dfb8bf84e6dc7c8))
* **release:** the chart guard can resume a release stranded between push and sign ([8092c3d](https://github.com/vaam-apps/vpay/commit/8092c3d8f06acc21b2146d673dfb8bf84e6dc7c8))


### Documentation

* charts/vpay:0.2.1 was deleted, so stop warning readers away from it ([#229](https://github.com/vaam-apps/vpay/issues/229)) ([74123da](https://github.com/vaam-apps/vpay/commit/74123da018b2d34e4a276227e5db3301b6a6bb80))
* **flows:** retire thirteen stale claims, most of them understating what exists ([#231](https://github.com/vaam-apps/vpay/issues/231)) ([7b3ebf9](https://github.com/vaam-apps/vpay/commit/7b3ebf93afd69572cf0455dcf7d4bfa00bed6a8a))
* retire ten stale claims, two of which would have cost an operator time ([#230](https://github.com/vaam-apps/vpay/issues/230)) ([5ab9199](https://github.com/vaam-apps/vpay/commit/5ab9199f6374248994e6c47b63c463d87db0d2b0))
* **status:** retire ten stale claims, and record implementation state on two ADRs ([#232](https://github.com/vaam-apps/vpay/issues/232)) ([a014463](https://github.com/vaam-apps/vpay/commit/a014463c5ca14247be3f5266a8119b7b8fb69f0f))
* the chart publishes and signs — and AGENTS.md is prettier-clean again ([8ad5050](https://github.com/vaam-apps/vpay/commit/8ad5050563fb5522b3492ce77d4c536d8f2f43d5))
* the chart publishes and signs, and AGENTS.md is prettier-clean again ([#228](https://github.com/vaam-apps/vpay/issues/228)) ([8ad5050](https://github.com/vaam-apps/vpay/commit/8ad5050563fb5522b3492ce77d4c536d8f2f43d5))

## [0.3.0](https://github.com/vaam-apps/vpay/compare/v0.2.2...v0.3.0) (2026-09-20)


### Features

* **release:** let release-please own the chart version too ([#225](https://github.com/vaam-apps/vpay/issues/225)) ([f5492e5](https://github.com/vaam-apps/vpay/commit/f5492e56610dfe07c1adb55238290b1b7cf88b1f))


### Bug Fixes

* **release:** give the chart job the docker login cosign actually reads ([#223](https://github.com/vaam-apps/vpay/issues/223)) ([5230136](https://github.com/vaam-apps/vpay/commit/523013630d33a584ffa23acf26f905e040679838))

## [0.2.2](https://github.com/vaam-apps/vpay/compare/v0.2.1...v0.2.2) (2026-09-20)


### Bug Fixes

* **ci:** guard fromJSON so a no-commit release-please run does not fail ([03e6bbf](https://github.com/vaam-apps/vpay/commit/03e6bbfad45a9aea3898df9daf915d70edb4d162))
* **ci:** pin every action in ci.yml to a commit SHA (DS-0002) ([#221](https://github.com/vaam-apps/vpay/issues/221)) ([a761bc7](https://github.com/vaam-apps/vpay/commit/a761bc7367976d76452dcad1c218e6d4114650e2))
* **ci:** scan the default branch on push, not just pull_request ([#220](https://github.com/vaam-apps/vpay/issues/220)) ([afc89b9](https://github.com/vaam-apps/vpay/commit/afc89b9ee7aba8811b60b8e4a10acc7099db2c6b))
* **erasure:** redact the last payment error pair and webhook response excerpt on customer erasure ([#211](https://github.com/vaam-apps/vpay/issues/211)) ([6cda795](https://github.com/vaam-apps/vpay/commit/6cda79568f89e1d1c77ccb837b00b13f84a4425d))


### Documentation

* the semver tag path is exercised, and publish-chart has been skipped once ([#222](https://github.com/vaam-apps/vpay/issues/222)) ([bb092c1](https://github.com/vaam-apps/vpay/commit/bb092c15d90dfa5c1df71bbdd56a505197451445))


### Continuous Integration

* **pnpm:** move settings and overrides to pnpm-workspace.yaml, pin pnpm 11 ([#217](https://github.com/vaam-apps/vpay/issues/217)) ([a8b5c0a](https://github.com/vaam-apps/vpay/commit/a8b5c0ad28b1d242e07fb1fe91f006f43dc6542c))
* **release:** publish by OIDC instead of a classic token ([#215](https://github.com/vaam-apps/vpay/issues/215)) ([a8d1764](https://github.com/vaam-apps/vpay/commit/a8d17641e91537b06e2b56e0dbd41add41ed92d9))
* **release:** publish the Helm chart to GHCR as an OCI artifact ([#219](https://github.com/vaam-apps/vpay/issues/219)) ([35848b5](https://github.com/vaam-apps/vpay/commit/35848b5cf8903738f3dba2bf34b1359dfcc74843))

## [0.2.1](https://github.com/vaam-apps/vpay/compare/v0.2.0...v0.2.1) (2026-09-19)


### Continuous Integration

* pin every action in release-please.yml to a commit SHA ([#213](https://github.com/vaam-apps/vpay/issues/213)) ([848c5b4](https://github.com/vaam-apps/vpay/commit/848c5b40d73b16f5562c21823d9b4c71520c4d9d))

## [0.2.0](https://github.com/vaam-apps/vpay/compare/v0.1.1...v0.2.0) (2026-09-18)


### Features

* GDPR personal-data inventory and its drift gate (issue [#144](https://github.com/vaam-apps/vpay/issues/144)) ([#187](https://github.com/vaam-apps/vpay/issues/187)) ([0799a8d](https://github.com/vaam-apps/vpay/commit/0799a8d27ac6d5069910ed586640f2d18be964b2))


### Bug Fixes

* **checkout,sdks/flutter:** a redirect rail's browser leg is a controlled surface ([#200](https://github.com/vaam-apps/vpay/issues/200)) ([bd5c85d](https://github.com/vaam-apps/vpay/commit/bd5c85d1fb5f1088f8d5836f0ba29f6ebcf89cc8)), closes [#195](https://github.com/vaam-apps/vpay/issues/195)
* **ci:** a bare-string extra-files entry reserialised Chart.yaml — use type: generic ([#204](https://github.com/vaam-apps/vpay/issues/204)) ([aeb9e24](https://github.com/vaam-apps/vpay/commit/aeb9e2427a52e43d49d6ab7b7d4cf068a6f39bf1))
* **ci:** master is still red on CHANGELOG.md and AGENTS.md, and the release gate has no tests ([#206](https://github.com/vaam-apps/vpay/issues/206)) ([5af959b](https://github.com/vaam-apps/vpay/commit/5af959b2330033aabc6f8c44f4758d9e5d6caf0f))
* **sdks/flutter:** read a remembered number back into the field and checkbox ([#197](https://github.com/vaam-apps/vpay/issues/197)) ([0bed092](https://github.com/vaam-apps/vpay/commit/0bed092c7d1bd19c67b43ea8a700231e901f19c6)), closes [#194](https://github.com/vaam-apps/vpay/issues/194)
* **sdks/flutter:** the native sheet offers the redirect back, instead of a false check your phone ([#208](https://github.com/vaam-apps/vpay/issues/208)) ([84143e1](https://github.com/vaam-apps/vpay/commit/84143e1de685718e7b9f6f46829db51ce8bef8e0))


### Tests

* macOS can run the sign-in suite, and two load flakes lose their race ([#210](https://github.com/vaam-apps/vpay/issues/210)) ([acdcbfe](https://github.com/vaam-apps/vpay/commit/acdcbfe40f2df32a9b7d49c08ee09c8001423ced))


### Continuous Integration

* adopt org-wide SAST, lint, Trivy and issue governance ([#207](https://github.com/vaam-apps/vpay/issues/207)) ([5f540b2](https://github.com/vaam-apps/vpay/commit/5f540b28e60b99ee0d7e10bcbc485a201747c09e))
* publish sdks/nodejs and sdks/stripe-js to npm on release ([#209](https://github.com/vaam-apps/vpay/issues/209)) ([7063037](https://github.com/vaam-apps/vpay/commit/7063037aa41d91bd85d19ba433ef100ca0770764))
* re-pin org reusable workflows for the MD024 changelog fix ([8659544](https://github.com/vaam-apps/vpay/commit/8659544196caa7f6dea74821e15c615e3e442330))
* re-pin org reusable workflows for the MD024 changelog fix ([43b8cfb](https://github.com/vaam-apps/vpay/commit/43b8cfb169dea0e93fb608e09015f4ed3fa702cf))
* re-pin org reusable workflows to current .github main ([#212](https://github.com/vaam-apps/vpay/issues/212)) ([369f2b2](https://github.com/vaam-apps/vpay/commit/369f2b2c58e28d1bf5454cdae549f9b0c41b0130))

## [0.1.1](https://github.com/vaam-apps/vpay/compare/v0.1.0...v0.1.1) (2026-09-17)


### Bug Fixes

* **ci:** release-please uses client-id, not the deprecated app-id ([#202](https://github.com/vaam-apps/vpay/issues/202)) ([a475a2d](https://github.com/vaam-apps/vpay/commit/a475a2d8de5c7b6d024c98ca04262d04e28c9195))


### Continuous Integration

* release-please proposes the version bump, and a gate for the pin that breaks the build ([#201](https://github.com/vaam-apps/vpay/issues/201)) ([5b82cbd](https://github.com/vaam-apps/vpay/commit/5b82cbd678624da5a9fd0f88027894c0a68700cb))
