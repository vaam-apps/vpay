# Changelog

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
