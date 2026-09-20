# Changelog

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
