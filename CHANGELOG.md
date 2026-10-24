# Changelog

## [0.13.0](https://github.com/vaam-apps/vpay/compare/v0.12.1...v0.13.0) (2026-10-24)


### Features

* **api:** a refund's destination may name the charge's payment-method type ([#363](https://github.com/vaam-apps/vpay/issues/363)) ([98f4d32](https://github.com/vaam-apps/vpay/commit/98f4d32ba8fdca60c86227b35db6a72ddf73f2e3))


### Bug Fixes

* **deps:** accept RUSTSEC-2026-0387 (crypto-bigint 0.5.5), documented; no release clears it ([#361](https://github.com/vaam-apps/vpay/issues/361)) ([55e848f](https://github.com/vaam-apps/vpay/commit/55e848fa1ec96d3015761b3eba2a14df43e6a704))
* **notchpay:** read a webhook's event name from `event`, as the sandbox sends it; record the first delivered webhooks ([#364](https://github.com/vaam-apps/vpay/issues/364)) ([4df5e0d](https://github.com/vaam-apps/vpay/commit/4df5e0d2b06cdfb847dac23243b238753afa6fc3))
* **storybook:** the built-stylesheet a11y case follows the theme's var() chain, and build-storybook runs it ([#349](https://github.com/vaam-apps/vpay/issues/349)) ([3bdbbea](https://github.com/vaam-apps/vpay/commit/3bdbbeacd2a98a6d5776d981140060971c1ce37d))

## [0.12.1](https://github.com/vaam-apps/vpay/compare/v0.12.0...v0.12.1) (2026-10-24)


### Bug Fixes

* **api:** a refund destination is keyed by the rail's code, and say so; correct the reconciler's Notch Pay registration claim ([#359](https://github.com/vaam-apps/vpay/issues/359)) ([1c8bd7b](https://github.com/vaam-apps/vpay/commit/1c8bd7b0bdd30b2cff395177a3a6e523dadbaddf))

## [0.12.0](https://github.com/vaam-apps/vpay/compare/v0.11.0...v0.12.0) (2026-10-24)


### Features

* **notchpay:** leave the account alone unless asked, and a wrong-network number is invalid_payer ([#353](https://github.com/vaam-apps/vpay/issues/353)) ([71a351d](https://github.com/vaam-apps/vpay/commit/71a351df4a06b0418a00d5e673d724bdf1546274))
* **provider:** verify rail callbacks, Notch Pay's signature first (RFC-0014) ([#357](https://github.com/vaam-apps/vpay/issues/357)) ([820c52b](https://github.com/vaam-apps/vpay/commit/820c52b3c84ae5b1b178b79c560e4912e5baf925))
* **routing:** the payer picks a network and a scored, explained router picks the rail (ADR-0029 D6, RFC-0008 revised) ([#346](https://github.com/vaam-apps/vpay/issues/346)) ([f0a2f7d](https://github.com/vaam-apps/vpay/commit/f0a2f7d7e90281ea40cfa99dcb82f57a7c7785c7))


### Bug Fixes

* **api:** the account-holder lookup asks for a rail code, not a method type; record the routing review's gaps ([#348](https://github.com/vaam-apps/vpay/issues/348)) ([82a68f4](https://github.com/vaam-apps/vpay/commit/82a68f4e4e81d0e45a17c22253ac28518373347a))
* **deps:** tokio 1.53.3 clears RUSTSEC-2026-0386 ([#358](https://github.com/vaam-apps/vpay/issues/358)) ([c122c86](https://github.com/vaam-apps/vpay/commit/c122c86d9ca6f5a1d91819572a71a6c7b6805b03))
* **deps:** zod 4.6.7 clears GHSA-mrhx-c7xc-m3w5 ([#352](https://github.com/vaam-apps/vpay/issues/352)) ([f115165](https://github.com/vaam-apps/vpay/commit/f1151654f58bc4689429affc5fc47057bb8fcde1))


### Documentation

* ADR-0032 accepted; RFC-0008's routing questions answered ([#351](https://github.com/vaam-apps/vpay/issues/351)) ([d01dd2a](https://github.com/vaam-apps/vpay/commit/d01dd2a47aa000cae70ce27718bcf64dbbbf1907))
* **notchpay:** measure the sandbox for the two-step split, the per-network channels and a webhook ([#350](https://github.com/vaam-apps/vpay/issues/350)) ([9499f09](https://github.com/vaam-apps/vpay/commit/9499f09dc175bd4464a0ab7426c40908323e5c30))
* **status:** retire three claims B3b and [#341](https://github.com/vaam-apps/vpay/issues/341) made stale ([#354](https://github.com/vaam-apps/vpay/issues/354)) ([14fc624](https://github.com/vaam-apps/vpay/commit/14fc62413b6a8df1382afe9744398e1b3e049fec))


### Tests

* **web:** the a11y gate's built-theme case resolves var() to a colour, and build-storybook runs it ([#355](https://github.com/vaam-apps/vpay/issues/355)) ([37fd72d](https://github.com/vaam-apps/vpay/commit/37fd72dff146be9cda03283fd859b978472f3bff))

## [0.11.0](https://github.com/vaam-apps/vpay/compare/v0.10.4...v0.11.0) (2026-10-23)


### Features

* **callbacks:** vpay registers Notch Pay's callback endpoint itself (RFC-0013 steps 2–4) ([#340](https://github.com/vaam-apps/vpay/issues/340)) ([8504536](https://github.com/vaam-apps/vpay/commit/8504536f117d8d2582b095d7726ab7a420961c30))
* **helm:** alert when no pass confirms a rail's callback endpoint; promtool tests for all six rules ([#343](https://github.com/vaam-apps/vpay/issues/343)) ([a008c05](https://github.com/vaam-apps/vpay/commit/a008c05a05c8fdae799a2c849c7b0552236de1c8))
* **provider:** a rail answer only a person can act on escalates at once ([#313](https://github.com/vaam-apps/vpay/issues/313), option 1) ([#341](https://github.com/vaam-apps/vpay/issues/341)) ([c672fff](https://github.com/vaam-apps/vpay/commit/c672fffc4f264ca4bac9d5aee7ae8bbaca058032))
* **provider:** a two-step rail's reference is committed before the prompt (ADR-0032) ([#345](https://github.com/vaam-apps/vpay/issues/345)) ([3633ca0](https://github.com/vaam-apps/vpay/commit/3633ca0c75bba652866f4ac39d834e1adf2963cb))


### Bug Fixes

* **deps:** ctutils 0.4.5 clears RUSTSEC-2026-0385 ([#344](https://github.com/vaam-apps/vpay/issues/344)) ([df9ef5c](https://github.com/vaam-apps/vpay/commit/df9ef5c0833e199c718de9f5afe44653dc685815))


### Documentation

* **notchpay:** run the timeout, canceled and Orange numbers through vpay (runbook B3b) ([#338](https://github.com/vaam-apps/vpay/issues/338)) ([e8e70ef](https://github.com/vaam-apps/vpay/commit/e8e70efe040eb11233f30979be2a34a752e10afd))
* **providers:** a desk study of Fapshi, and its row in the matrix ([#342](https://github.com/vaam-apps/vpay/issues/342)) ([be31abc](https://github.com/vaam-apps/vpay/commit/be31abcd0c1ca36e92f7b1044c916ad321b16094))

## [0.10.4](https://github.com/vaam-apps/vpay/compare/v0.10.3...v0.10.4) (2026-10-22)


### Bug Fixes

* **xtask:** exempt MTN's sandbox test MSISDN from verify-citations in the two docs that name it ([#335](https://github.com/vaam-apps/vpay/issues/335)) ([5cd22c5](https://github.com/vaam-apps/vpay/commit/5cd22c5c1857e9c7581f971ee37009136f775874))


### Documentation

* **rfc:** RFC-0009 revised: one company first, Stripe Connect's shape later ([#336](https://github.com/vaam-apps/vpay/issues/336)) ([facbb67](https://github.com/vaam-apps/vpay/commit/facbb67769ac0c252e8e3f463c66993c58b80cda))

## [0.10.3](https://github.com/vaam-apps/vpay/compare/v0.10.2...v0.10.3) (2026-10-22)


### Documentation

* correct fourteen places vpay contradicted itself at v0.10.2 ([#333](https://github.com/vaam-apps/vpay/issues/333)) ([e7406dd](https://github.com/vaam-apps/vpay/commit/e7406ddadf822d6f6b0512d2bd97cc331c69ad21))
* **flows:** link the callback human tests ([#330](https://github.com/vaam-apps/vpay/issues/330), [#331](https://github.com/vaam-apps/vpay/issues/331)) from the "To confirm" items ([#332](https://github.com/vaam-apps/vpay/issues/332)) ([45a244f](https://github.com/vaam-apps/vpay/commit/45a244fc1441a9fdb52cc39ced7ead0716d6b430))
* scan:live is every ten minutes, and MTN documents no IP-allowlisted callback host (RFC-0013 step 0, items 4-5) ([#326](https://github.com/vaam-apps/vpay/issues/326)) ([1b2543b](https://github.com/vaam-apps/vpay/commit/1b2543bbdbc7f74b13f39bb89f2ecc25765ba086))
* **status:** the Flutter example lockfile sync is confirmed by three releases ([#329](https://github.com/vaam-apps/vpay/issues/329)) ([4f3344e](https://github.com/vaam-apps/vpay/commit/4f3344e1d2681486b2d2a0b43a8e1c8a5529ef60))


### Tests

* **notchpay:** probe POST /refunds in the sandbox; it answers 403 (RFC-0012 § 10 items 6, 12) ([#334](https://github.com/vaam-apps/vpay/issues/334)) ([9086ba4](https://github.com/vaam-apps/vpay/commit/9086ba450747eff5ab2b88b23cc41850926ef0cf))

## [0.10.2](https://github.com/vaam-apps/vpay/compare/v0.10.1...v0.10.2) (2026-10-21)


### Bug Fixes

* **api:** a declined confirm fails the charge before it records the attempt ([#322](https://github.com/vaam-apps/vpay/issues/322)) ([b7cc6c9](https://github.com/vaam-apps/vpay/commit/b7cc6c96bf5ea99c862997222743a8e15ead6251))
* **deps:** Storybook 10.6.3 and vitest 4.1.12 clear three advisories that turned web red ([#325](https://github.com/vaam-apps/vpay/issues/325)) ([a522b86](https://github.com/vaam-apps/vpay/commit/a522b86dd4becaa761a97aa18daa6ec060d24ffe))

## [0.10.1](https://github.com/vaam-apps/vpay/compare/v0.10.0...v0.10.1) (2026-10-21)


### Bug Fixes

* **dashboard:** the theme script runs first in body, and the re-sign-in lands on /overview ([#323](https://github.com/vaam-apps/vpay/issues/323)) ([63c395d](https://github.com/vaam-apps/vpay/commit/63c395dfaf27e44eb6f943969dc08f59d7696e43))


### Documentation

* **callbacks:** no rail is documented to retry a callback; MTN PUT/POST and Orange order_id recorded as open (RFC-0013 step 0, items 1-3) ([#319](https://github.com/vaam-apps/vpay/issues/319)) ([0706cd5](https://github.com/vaam-apps/vpay/commit/0706cd57f0baf57bc5a9fbf43b8c1cb058e5bd38))
* **callbacks:** no rail is documented to retry a callback; record MTN's PUT/POST and Orange's order_id as open (RFC-0013 step 0, items 1-3) ([0706cd5](https://github.com/vaam-apps/vpay/commit/0706cd57f0baf57bc5a9fbf43b8c1cb058e5bd38))

## [0.10.0](https://github.com/vaam-apps/vpay/compare/v0.9.1...v0.10.0) (2026-10-21)


### ⚠ BREAKING CHANGES

* **sdks:** these refusals are no longer a TypeError, and cannot be. A class has one superclass, and making VpayError extend TypeError would make every error the package throws a TypeError. Code that tested `instanceof TypeError` for them should test `instanceof VpayInvalidParamsError`. No signature changed, and nothing that was sent before is refused now.
* **dashboard:** Material 3 Expressive redesign, Overview home, and no more page reloads ([#318](https://github.com/vaam-apps/vpay/issues/318))

### Features

* **dashboard:** Material 3 Expressive redesign, Overview home, and no more page reloads ([#318](https://github.com/vaam-apps/vpay/issues/318)) ([73c3a7c](https://github.com/vaam-apps/vpay/commit/73c3a7cf471cc4d612fa78faefd85d7b1991afc8))


### Bug Fixes

* **sdks:** every Node SDK local refusal is a VpayInvalidParamsError ([#315](https://github.com/vaam-apps/vpay/issues/315)) ([8796705](https://github.com/vaam-apps/vpay/commit/879670598ed3411116230872efa6d407e35be104))
* **worker:** record a refused status read as answered, a refused submit as unresolved ([#314](https://github.com/vaam-apps/vpay/issues/314)) ([6d9dfbf](https://github.com/vaam-apps/vpay/commit/6d9dfbf5e56a8561d6f4ee3185225cdac0814ef6))


### Documentation

* **notchpay:** canceled is payer_declined in the risk list and failures tables ([#312](https://github.com/vaam-apps/vpay/issues/312)) ([73a5b5a](https://github.com/vaam-apps/vpay/commit/73a5b5af67efba91437f07375799fa8a5eda69f7))
* **notchpay:** RFC-0012 step 5 against Notch Pay's real sandbox ([#310](https://github.com/vaam-apps/vpay/issues/310)) ([e28292d](https://github.com/vaam-apps/vpay/commit/e28292d3a96b7f5fa364da1a5e7140ea08faecf9))
* **status:** vpay is in live testing on some clusters, not a scaffold ([#308](https://github.com/vaam-apps/vpay/issues/308)) ([4ca1fed](https://github.com/vaam-apps/vpay/commit/4ca1fed0672c1a40511a04d77b489eee92318649))


### Tests

* **dashboard:** regression tests for the SideNav duplicate key ([#163](https://github.com/vaam-apps/vpay/issues/163)) ([#274](https://github.com/vaam-apps/vpay/issues/274)) ([865420b](https://github.com/vaam-apps/vpay/commit/865420ba52af1efadcbac6793b3a05d7d97d78ab))
* **sdks:** pin that no payer surface puts a dot segment in a path ([#309](https://github.com/vaam-apps/vpay/issues/309)) ([f554f87](https://github.com/vaam-apps/vpay/commit/f554f87160c36409024f486e5c3d9199c9db8a1b))
* **vpay-server:** give each cli test server its own observability port ([#311](https://github.com/vaam-apps/vpay/issues/311)) ([71295d1](https://github.com/vaam-apps/vpay/commit/71295d1649ac6e0461e6795072b59e0fd7cc4a04))
* **vpay-server:** make a failed boot in tests/cli.rs say why ([#317](https://github.com/vaam-apps/vpay/issues/317)) ([8db838d](https://github.com/vaam-apps/vpay/commit/8db838d6ad81559cfe035b2dc7aefb09e0ddcf3b))

## [0.9.1](https://github.com/vaam-apps/vpay/compare/v0.9.0...v0.9.1) (2026-10-21)


### Bug Fixes

* **notchpay:** address process and read by Notch Pay's trx. reference ([#304](https://github.com/vaam-apps/vpay/issues/304)) ([5b4c583](https://github.com/vaam-apps/vpay/commit/5b4c583cd8987a91b8fe92130cdab7687038ffaa))
* **sdks:** refuse an empty or all-dots id before any request ([#306](https://github.com/vaam-apps/vpay/issues/306)) ([1ffd148](https://github.com/vaam-apps/vpay/commit/1ffd14840b6466c65272be4c3e8970c06ff47ae3))


### Documentation

* **notchpay:** RFC-0012 step 1 against Notch Pay's real sandbox; refuse a short "complete" ([#303](https://github.com/vaam-apps/vpay/issues/303)) ([f7c7ae3](https://github.com/vaam-apps/vpay/commit/f7c7ae33c3aeb7febb2ca70cc015f6f0c80be52d))

## [0.9.0](https://github.com/vaam-apps/vpay/compare/v0.8.0...v0.9.0) (2026-10-20)


### Features

* **adapters:** the notchpay rail, admitted under ADR-0031 (rule 3 amended) ([#295](https://github.com/vaam-apps/vpay/issues/295)) ([357c825](https://github.com/vaam-apps/vpay/commit/357c82556d8dff277751ed9bd65b0488b6719b40))
* **notchpay:** opt-in --ca-bundle for the sandbox probe (cloud sessions only) ([#302](https://github.com/vaam-apps/vpay/issues/302)) ([d14cbe9](https://github.com/vaam-apps/vpay/commit/d14cbe983330a69bd3c8bc4ac0b72e456344e250))


### Bug Fixes

* **config:** Notch Pay's real test keys carry `test.`, not `test_` ([#298](https://github.com/vaam-apps/vpay/issues/298)) ([18ef9b7](https://github.com/vaam-apps/vpay/commit/18ef9b7123cc24f6aeaf25a742a741baad8da79d))
* **deps:** hono 4.13.13 clears GHSA-5r4p-p66f-jhc7 ([#300](https://github.com/vaam-apps/vpay/issues/300)) ([a38937a](https://github.com/vaam-apps/vpay/commit/a38937a35dbdd0c84f213b9b4abc424dd87b528e))
* **provider:** path_segment refuses an empty or all-dots segment ([#294](https://github.com/vaam-apps/vpay/issues/294)) ([de9e2bf](https://github.com/vaam-apps/vpay/commit/de9e2bf731e12ffd181a27d593978a21352cbb65))


### Documentation

* correct real MTN sandbox payment totals ([#283](https://github.com/vaam-apps/vpay/issues/283)) ([cd25532](https://github.com/vaam-apps/vpay/commit/cd255325cddc2876e16c4301b760caf84feb6ade))
* **rfc:** Notch Pay (RFC-0012), self-registered callbacks (RFC-0013), the provider matrix, and an unlinked Notch Pay client ([#285](https://github.com/vaam-apps/vpay/issues/285)) ([b8be107](https://github.com/vaam-apps/vpay/commit/b8be107c8896c91ccdf433ad67c521e9d871992e))
* **rfc:** RFC-0014 — verifying rail callbacks ([#296](https://github.com/vaam-apps/vpay/issues/296)) ([0977616](https://github.com/vaam-apps/vpay/commit/097761669e6810d4546e7376ebd4610d7c4457c8))
* **runbooks:** a Notch Pay sandbox runbook, a probe, and a step-5 profile ([#299](https://github.com/vaam-apps/vpay/issues/299)) ([c5b0a80](https://github.com/vaam-apps/vpay/commit/c5b0a80222daa154f0c8dab6071d42680518577b))
* Valantine Suh ran the 2026-09-15 MTN sandbox tests, not the maintainer ([#297](https://github.com/vaam-apps/vpay/issues/297)) ([72a31a7](https://github.com/vaam-apps/vpay/commit/72a31a780090cbf6027620f762c006a023edf2a4))

## [0.8.0](https://github.com/vaam-apps/vpay/compare/v0.7.0...v0.8.0) (2026-10-20)


### Features

* **dashboard:** staff sign-in through an OpenID Connect provider (ADR-0030) ([#280](https://github.com/vaam-apps/vpay/issues/280)) ([b69e14d](https://github.com/vaam-apps/vpay/commit/b69e14d1c1bd0bc276f8b339c49f1d3732e6caaa))


### Bug Fixes

* **deps:** rustls 0.23.46 for RUSTSEC-2026-0346..0349 ([#281](https://github.com/vaam-apps/vpay/issues/281)) ([0a86e83](https://github.com/vaam-apps/vpay/commit/0a86e834f3fdf52b7f625c3522bd1778fc3f60cd))

## [0.7.0](https://github.com/vaam-apps/vpay/compare/v0.6.0...v0.7.0) (2026-10-18)


### Features

* **refunds:** name the payment a refund unwinds, and guard the Origin arm end to end ([c8c1904](https://github.com/vaam-apps/vpay/commit/c8c19041bfbbecbf76b373b1c620861a7a7859dd))
* **refunds:** name the payment a refund unwinds, and guard the Origin arm end to end (RFC-0010 step 1) ([#277](https://github.com/vaam-apps/vpay/issues/277)) ([c8c1904](https://github.com/vaam-apps/vpay/commit/c8c19041bfbbecbf76b373b1c620861a7a7859dd))


### Bug Fixes

* **frontends:** numeric USER so Kubernetes runAsNonRoot can verify the dashboard and checkout images ([#278](https://github.com/vaam-apps/vpay/issues/278)) ([ca26054](https://github.com/vaam-apps/vpay/commit/ca260547017e33faad243d7d2585e4706e145b19))


### Documentation

* RFC-0009, RFC-0010, RFC-0011, and ADR-0029 accepting RFC-0004 § 7 and RFC-0008 ([#275](https://github.com/vaam-apps/vpay/issues/275)) ([6524032](https://github.com/vaam-apps/vpay/commit/6524032f1a1bd32de6a2fa0fcbc641085d1cf16b))

## [0.6.0](https://github.com/vaam-apps/vpay/compare/v0.5.0...v0.6.0) (2026-10-09)


### ⚠ BREAKING CHANGES

* vpay-sdk (sdks/rust) — ListPaymentIntentsParams, ListCheckoutSessionsParams and ListRefundsParams each gained a public `customer: Option<String>` field (in 8d328d4). Code that builds any of the three with a struct literal naming every field and no `..Default::default()` no longer compiles (E0063, missing field `customer`); add `customer: None` or `..Default::default()`. Code using Default::default() or field assignment, and every @vaam-apps/vpay-sdk caller, is unaffected.

### Features

* **checkout:** write a session's customer onto a customer-less intent (ADR-0025) ([#253](https://github.com/vaam-apps/vpay/issues/253)) ([fec2fc2](https://github.com/vaam-apps/vpay/commit/fec2fc230b28867b064c964003c0d48e4f451c80))
* customer filters on list endpoints and manual invoice payments (RFC-0004 step A) ([#251](https://github.com/vaam-apps/vpay/issues/251)) ([b747e5d](https://github.com/vaam-apps/vpay/commit/b747e5d5068c4fda6b6b1b9aa06041ddc32230d3))
* **dashboard:** move the dashboard to @vaam-apps/ui 0.4.0 ([#258](https://github.com/vaam-apps/vpay/issues/258)) ([f68fda0](https://github.com/vaam-apps/vpay/commit/f68fda097c66299fda183686288cadb6c8198955))
* **privacy:** protected diagnostic representations and the debug_protections gate (RFC-0002 PR 3, issue [#147](https://github.com/vaam-apps/vpay/issues/147)) ([#273](https://github.com/vaam-apps/vpay/issues/273)) ([69d822a](https://github.com/vaam-apps/vpay/commit/69d822ab243011e8e834fc8fecb7ebc0eb2e3408))


### Bug Fixes

* **dashboard:** the timeline note names only the two unwritten event types ([#247](https://github.com/vaam-apps/vpay/issues/247)) ([5129f60](https://github.com/vaam-apps/vpay/commit/5129f60c32701f130ce798ec7e1f1c21c2cfef7f))
* **deps:** clear the next, brace-expansion, sharp and source-map-js advisories ([#272](https://github.com/vaam-apps/vpay/issues/272)) ([16323c0](https://github.com/vaam-apps/vpay/commit/16323c0313ec5a963103f83571a629f6d653640f))
* **erasure:** reach a payer's payments through their checkout sessions too (ADR-0027) ([#257](https://github.com/vaam-apps/vpay/issues/257)) ([1bba541](https://github.com/vaam-apps/vpay/commit/1bba54131e9d605052cf0f3892dbc9230cd49df9))
* **helm:** accept Helm's global key so the chart works as a subchart ([#270](https://github.com/vaam-apps/vpay/issues/270)) ([8cad7d1](https://github.com/vaam-apps/vpay/commit/8cad7d1400eff2135239a0f4618ef1648b7a33a7))
* **justfile:** make `just migrations-manifest` run on macOS ([9184e42](https://github.com/vaam-apps/vpay/commit/9184e42e68eb44c70ad27e5b9942d2a3378ca2f8))
* **justfile:** make just migrations-manifest run on macOS ([#252](https://github.com/vaam-apps/vpay/issues/252)) ([9184e42](https://github.com/vaam-apps/vpay/commit/9184e42e68eb44c70ad27e5b9942d2a3378ca2f8))
* **op:** keep a spent client-assertion jti past the validator's leeway (ADR-0028) ([#271](https://github.com/vaam-apps/vpay/issues/271)) ([87166ea](https://github.com/vaam-apps/vpay/commit/87166eafb71eb105ff90a9958377866840447748))
* **worker:** schedule every job on the database's clock, never the app host's (ADR-0026) ([#256](https://github.com/vaam-apps/vpay/issues/256)) ([434dda7](https://github.com/vaam-apps/vpay/commit/434dda7cc00d2e1a109c83e9b9fb10d6b2088366))


### Documentation

* **adr:** ADR-0024 — customer filters and manual payments (RFC-0004 step A) ([#248](https://github.com/vaam-apps/vpay/issues/248)) ([51c3c38](https://github.com/vaam-apps/vpay/commit/51c3c38dc852b8368c9805c3d9dee24189f09cc5))
* retire thirteen stale claims found re-verifying the skills, and make the route probe self-checking ([#255](https://github.com/vaam-apps/vpay/issues/255)) ([1d20640](https://github.com/vaam-apps/vpay/commit/1d20640677c13218136bf5f52cab6faba64d5e03))


### Tests

* read "now" off the database in fixtures that Postgres' now() judges ([#254](https://github.com/vaam-apps/vpay/issues/254)) ([d08dafd](https://github.com/vaam-apps/vpay/commit/d08dafdd1e833c23cbaad7fe95b9613738925a86))


### Chores

* **deps:** bump @vaam-apps/ui to v0.2.4 ([#249](https://github.com/vaam-apps/vpay/issues/249)) ([c95e257](https://github.com/vaam-apps/vpay/commit/c95e2573d463b058b7768caa2848bf8231e7f121))
* **deps:** bump cratestack 0.12.0 to 0.15.0 ([#259](https://github.com/vaam-apps/vpay/issues/259)) ([a33aac6](https://github.com/vaam-apps/vpay/commit/a33aac61029c5e2d81e77217cd9ea23d43f946ac))

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
