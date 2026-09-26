# Changelog

## [0.7.0](https://github.com/japboy/lens/compare/v0.6.0...v0.7.0) (2026-09-26)


### Features

* **acp:** add manual updates and refine agent settings ([#113](https://github.com/japboy/lens/issues/113)) ([10378a6](https://github.com/japboy/lens/commit/10378a627f6a6dd767e07288acac03b98f83f090))
* **overlay:** retain session response history ([#130](https://github.com/japboy/lens/issues/130)) ([604760b](https://github.com/japboy/lens/commit/604760b8cef4d1c408256e94794ed4290a6e7a4c))


### Bug Fixes

* **agent:** finalize session execution state ([#127](https://github.com/japboy/lens/issues/127)) ([f8b98f2](https://github.com/japboy/lens/commit/f8b98f2b1a36ca896359b66e2d89159d5ba4de72))
* **agents:** detach environment resolver from tty ([#132](https://github.com/japboy/lens/issues/132)) ([d188e78](https://github.com/japboy/lens/commit/d188e78867ce44148e08c35af562e03c47d09eff))
* **agents:** refresh defaults after adapter updates ([#134](https://github.com/japboy/lens/issues/134)) ([8b5f9d8](https://github.com/japboy/lens/commit/8b5f9d8d6d23273c667850014bda79a9c23c6145))
* **ci:** skip verification for closed pull requests ([#135](https://github.com/japboy/lens/issues/135)) ([9334f94](https://github.com/japboy/lens/commit/9334f9489a4337a83f9bb831a727e95084b11fb6))
* **deps:** centralize the reqwest version ([#128](https://github.com/japboy/lens/issues/128)) ([7bbcc19](https://github.com/japboy/lens/commit/7bbcc19712d40b51b7e825bae9f860f925237bbb))
* **history:** preserve the selected agent ([#133](https://github.com/japboy/lens/issues/133)) ([0024ebe](https://github.com/japboy/lens/commit/0024ebef512e4f950cc47918f5f17f6fafe98143))
* **macos:** reject mismatched singleton windows ([#124](https://github.com/japboy/lens/issues/124)) ([90f1bb1](https://github.com/japboy/lens/commit/90f1bb1924d0901e8c24241867c2e6e9af9afb96)), closes [#114](https://github.com/japboy/lens/issues/114)
* **release:** simplify artifact lifecycle ([#125](https://github.com/japboy/lens/issues/125)) ([ed68f0d](https://github.com/japboy/lens/commit/ed68f0d21fba27a1178d9ac1691f938d2e57b18b)), closes [#116](https://github.com/japboy/lens/issues/116) [#117](https://github.com/japboy/lens/issues/117) [#118](https://github.com/japboy/lens/issues/118)
* **runtime:** clean up cancelled staging ([#126](https://github.com/japboy/lens/issues/126)) ([e03ce6c](https://github.com/japboy/lens/commit/e03ce6c38e74a80fc43c2742481e3224d60bdb8b)), closes [#115](https://github.com/japboy/lens/issues/115)
* **ui:** align macOS controls with system colors ([#136](https://github.com/japboy/lens/issues/136)) ([794b1ed](https://github.com/japboy/lens/commit/794b1edc54fdedb7421cc4c91920e62bdcaa46a8))

## [0.6.0](https://github.com/japboy/lens/compare/v0.5.0...v0.6.0) (2026-09-20)


### Features

* **app:** confirm quit during Agent work ([#112](https://github.com/japboy/lens/issues/112)) ([e1b696b](https://github.com/japboy/lens/commit/e1b696b1c78edfd7d70f576e931c14001568737f))
* **history:** persist sessions and unify Agent selection ([#109](https://github.com/japboy/lens/issues/109)) ([776ae47](https://github.com/japboy/lens/commit/776ae476960ab059b63d3a80c3a606e08db27916))


### Bug Fixes

* **release:** preserve durable recovery evidence ([#106](https://github.com/japboy/lens/issues/106)) ([74ac4d9](https://github.com/japboy/lens/commit/74ac4d9804db2e90fdaea919b7f757a107f4cccd))
* **ui:** foreground newly opened windows ([#111](https://github.com/japboy/lens/issues/111)) ([d09cf04](https://github.com/japboy/lens/commit/d09cf04c7f3a2341b2a3acc0a398140ba38a0193))

## [0.5.0](https://github.com/japboy/lens/compare/v0.4.2...v0.5.0) (2026-09-20)


### Features

* **agent:** support external ACP agent presets ([#103](https://github.com/japboy/lens/issues/103)) ([6979ec4](https://github.com/japboy/lens/commit/6979ec4f621f110ce390891cd854de18b83b5ae7))

## [0.4.2](https://github.com/japboy/lens/compare/v0.4.1...v0.4.2) (2026-09-18)


### Bug Fixes

* **agent:** resolve initial working environments ([#102](https://github.com/japboy/lens/issues/102)) ([7f5b7c2](https://github.com/japboy/lens/commit/7f5b7c27d4ce00683f8d889a4817d817e45eee27)), closes [#97](https://github.com/japboy/lens/issues/97)
* **history:** retain latest successful output ([#98](https://github.com/japboy/lens/issues/98)) ([cdbd881](https://github.com/japboy/lens/commit/cdbd8814e02a472cc214bc19dc89f0180d4c4f8f))

## [0.4.1](https://github.com/japboy/lens/compare/v0.4.0...v0.4.1) (2026-09-18)


### Bug Fixes

* **history:** restore Claude HTML previews ([#93](https://github.com/japboy/lens/issues/93)) ([613e74a](https://github.com/japboy/lens/commit/613e74a4da1d835face018a01519e1d8cc2fda96))

## [0.4.0](https://github.com/japboy/lens/compare/v0.3.0...v0.4.0) (2026-09-18)


### Features

* add read-only ACP session history ([#74](https://github.com/japboy/lens/issues/74)) ([e20a1da](https://github.com/japboy/lens/commit/e20a1da40984056e4369ef2980de3b34b26bd6cc))
* **math:** render LaTeX in Markdown and HTML ([#77](https://github.com/japboy/lens/issues/77)) ([693f88f](https://github.com/japboy/lens/commit/693f88fa2163eeafce6ddeae054c0d3ef651cd25))
* **prompts:** refine presets and add Evocative ([#76](https://github.com/japboy/lens/issues/76)) ([ef92dee](https://github.com/japboy/lens/commit/ef92deee52a1f920b183ceb56e2ed93d5c267d78))
* **settings:** add Screen & System Audio Recording ([#92](https://github.com/japboy/lens/issues/92)) ([8628f0d](https://github.com/japboy/lens/commit/8628f0d592b1a74644c4759edfa58db1dc8e97ac))


### Bug Fixes

* **ci:** remove dependency graph approval gate ([#90](https://github.com/japboy/lens/issues/90)) ([443345d](https://github.com/japboy/lens/commit/443345d4e96adee94bed89edb3e61ac038dca9bb))
* **deps:** update Cytoscape to 3.34.3 with consistent lock metadata ([#78](https://github.com/japboy/lens/issues/78)) ([71813a0](https://github.com/japboy/lens/commit/71813a0d44df407f6afca1f23aa64944eb3063f4))
* **deps:** update dependency mermaid to v12 ([#89](https://github.com/japboy/lens/issues/89)) ([5d71547](https://github.com/japboy/lens/commit/5d7154785c20f688f3fa329985ff735c527429a8))
* **deps:** update tauri dependencies ([#80](https://github.com/japboy/lens/issues/80)) ([425e64e](https://github.com/japboy/lens/commit/425e64e6168e27462e583c94bb03d10b12c6c5d4))
* **settings:** use app identifier for storage ([#79](https://github.com/japboy/lens/issues/79)) ([b1c4490](https://github.com/japboy/lens/commit/b1c44903fc51933c5d8bd7705742d2b8a79e73af))

## [0.3.0](https://github.com/japboy/lens/compare/v0.2.0...v0.3.0) (2026-09-10)


### Features

* **acp:** update registry adapters without HTML patches ([#69](https://github.com/japboy/lens/issues/69)) ([5f54007](https://github.com/japboy/lens/commit/5f540077c44d1ed85648ee3a7e9784796f8916d5))
* **settings:** add editable learner prompt presets ([#70](https://github.com/japboy/lens/issues/70)) ([de85302](https://github.com/japboy/lens/commit/de853026b41b9bdcc869583033cde680fd6e49b0))


### Bug Fixes

* **ci:** verify the workflow-selected release action ([#64](https://github.com/japboy/lens/issues/64)) ([bd7141b](https://github.com/japboy/lens/commit/bd7141b01003e6485e94708e999ae5ece6419f90))
* **release:** synchronize workspace versions ([#71](https://github.com/japboy/lens/issues/71)) ([fc2a646](https://github.com/japboy/lens/commit/fc2a6462ac716035a0f385ae03cee8c63417f98a))
* **runtime:** unify development and managed Node versions ([#60](https://github.com/japboy/lens/issues/60)) ([ccb6f91](https://github.com/japboy/lens/commit/ccb6f9136f073131aa5816b1e6570fc94338b35f))

## [0.2.0](https://github.com/japboy/lens/compare/v0.1.0...v0.2.0) (2026-09-08)


### Features

* **desktop:** expand output media with the Fullscreen API ([#52](https://github.com/japboy/lens/issues/52)) ([ee4c324](https://github.com/japboy/lens/commit/ee4c324ce198c6a9d508406de7ff981f13993a91))
* **overlay:** publish and render HTML through bundled MCP ([#57](https://github.com/japboy/lens/issues/57)) ([9b1b38e](https://github.com/japboy/lens/commit/9b1b38eb44283ecacae503e28875b65c3b59806d))


### Bug Fixes

* **deps:** update acp dependencies ([#50](https://github.com/japboy/lens/issues/50)) ([0dbb730](https://github.com/japboy/lens/commit/0dbb73039f581bb0a4a0ef02a8f8bcc30a197ce5))
* **settings:** restore saved dynamic Agent selections ([#56](https://github.com/japboy/lens/issues/56)) ([8c4b5a7](https://github.com/japboy/lens/commit/8c4b5a75ef7454c295bf634ef78970af0ae89001))

## 0.1.0 (2026-09-06)


### Features

* **acp:** render image output blocks ([#8](https://github.com/japboy/lens/issues/8)) ([b38463e](https://github.com/japboy/lens/commit/b38463e1a2c0434f3d29106f804286ae75880207))
* **agent:** add shared defaults and session interaction controls ([#39](https://github.com/japboy/lens/issues/39)) ([d5917e8](https://github.com/japboy/lens/commit/d5917e8a9e215dbd608c14004d6cac6fde987d56))
* **desktop:** add About window and bundled license notices ([#44](https://github.com/japboy/lens/issues/44)) ([383e7fd](https://github.com/japboy/lens/commit/383e7fd5a9c5496b68e79c9737a4a9ff58e958f6))
* **desktop:** prerender progressive windows with Lit DSD ([#51](https://github.com/japboy/lens/issues/51)) ([98f6a74](https://github.com/japboy/lens/commit/98f6a7474885514528a6785969dedc2a556a6eb7))
* **icons:** adopt canonical Lens icon resources ([#35](https://github.com/japboy/lens/issues/35)) ([7a94750](https://github.com/japboy/lens/commit/7a9475073d495b0d89ab9909a1466f08156b45e1))
* **lens:** make window geometry user-owned ([#11](https://github.com/japboy/lens/issues/11)) ([4ee33ef](https://github.com/japboy/lens/commit/4ee33efd60485fab6ff8afe5fea2d1a128d358d1))
* **lens:** present interpretation media in a hero carousel ([#36](https://github.com/japboy/lens/issues/36)) ([23687c2](https://github.com/japboy/lens/commit/23687c231df65454cd987a5de59c4740934a38f5))
* **lens:** refine source and overlay layout ([#9](https://github.com/japboy/lens/issues/9)) ([e4bc52f](https://github.com/japboy/lens/commit/e4bc52fba88300c926a2b4919752a1b993238733))
* **lens:** refine target window presentation ([#23](https://github.com/japboy/lens/issues/23)) ([0f483bf](https://github.com/japboy/lens/commit/0f483bf9793af097742d354927ee7fd6f4434180))
* **lens:** refresh overlay presentation ([#27](https://github.com/japboy/lens/issues/27)) ([fc9b23e](https://github.com/japboy/lens/commit/fc9b23e5520149a07a39636785712a9196faf6da))
* **lens:** structure multimodal AX input ([#4](https://github.com/japboy/lens/issues/4)) ([d31e523](https://github.com/japboy/lens/commit/d31e5232d6e43d18a80a69b18d9d3ebf9fa4f9f7))
* **lens:** support multi-window target selection ([#20](https://github.com/japboy/lens/issues/20)) ([803cee2](https://github.com/japboy/lens/commit/803cee2a4c3c1c2289819b08b6268603c74273bc))
* **lens:** synchronize observed source updates ([#29](https://github.com/japboy/lens/issues/29)) ([2df4401](https://github.com/japboy/lens/commit/2df44019b94afbc0acd93f0dd6721ffb77b579d8))
* **markdown:** render Mermaid diagrams ([#7](https://github.com/japboy/lens/issues/7)) ([97e49f3](https://github.com/japboy/lens/commit/97e49f3b762cc3faa5c5665c8a99e677b3646411))
* **notifications:** show Agent progress and inline response controls ([#42](https://github.com/japboy/lens/issues/42)) ([415f0fb](https://github.com/japboy/lens/commit/415f0fb6d42a6af3e216d5d5c786f8de1a64aa65))
* **release:** automate verified ad-hoc macOS DMG releases ([#43](https://github.com/japboy/lens/issues/43)) ([3069e2b](https://github.com/japboy/lens/commit/3069e2bf1b645b8f75750a1fadcde6bf4434d778))
* **settings:** adapt macOS text entry ([#10](https://github.com/japboy/lens/issues/10)) ([f2d82a5](https://github.com/japboy/lens/commit/f2d82a5f9913dc0cab681bc615ce1f037f85adf1))
* **settings:** adopt grouped macOS presentation ([#14](https://github.com/japboy/lens/issues/14)) ([421a15a](https://github.com/japboy/lens/commit/421a15aafbda6f470c3a9cdfdb5cbcb7a6cc0bf5))
* **settings:** expose agent prompt template ([#32](https://github.com/japboy/lens/issues/32)) ([fa55b33](https://github.com/japboy/lens/commit/fa55b330fd41b925001267e474f6d9ea70c44637))
* **settings:** make Agent Prompt editable ([#6](https://github.com/japboy/lens/issues/6)) ([5b3a579](https://github.com/japboy/lens/commit/5b3a5791cba217507783cb4cbe7d9e37f4626132))


### Bug Fixes

* **agent:** preserve initial ACP session configuration ([#37](https://github.com/japboy/lens/issues/37)) ([b2b41bf](https://github.com/japboy/lens/commit/b2b41bf023260dcd9b3e1f8918a943f9f750ef83)), closes [#15](https://github.com/japboy/lens/issues/15)
* **agent:** render generated tool images ([#33](https://github.com/japboy/lens/issues/33)) ([3097919](https://github.com/japboy/lens/commit/30979195d88c3f67ef3e1603714a363ce2fe03f1))
* **bundle:** apply ad hoc macOS signing ([#22](https://github.com/japboy/lens/issues/22)) ([895f6ee](https://github.com/japboy/lens/commit/895f6eecc602849bee092c1b4dbed7d259e119ef))
* **ci:** use the canonical mise release tag ([#3](https://github.com/japboy/lens/issues/3)) ([d0d84a4](https://github.com/japboy/lens/commit/d0d84a42732ebbb32198c68f007344711f46fc52))
* **lens:** refresh selected window facts ([#31](https://github.com/japboy/lens/issues/31)) ([671d910](https://github.com/japboy/lens/commit/671d91040a0cda60c294fb225058671653b0a5b3))
* **macos:** activate lazy accessibility targets ([#5](https://github.com/japboy/lens/issues/5)) ([4c94412](https://github.com/japboy/lens/commit/4c9441246a593ed57a10f41f90333179e9dcd69d))
* **mermaid:** allow safe HTML labels ([#30](https://github.com/japboy/lens/issues/30)) ([a836fd0](https://github.com/japboy/lens/commit/a836fd0f3b36b9d0aa0d7a343db9587cce6fde79))
* **release:** discover pending releases through pull requests ([#45](https://github.com/japboy/lens/issues/45)) ([190146a](https://github.com/japboy/lens/commit/190146ac34583ce6ccebe5709ac6d4fb81b90b1c))
* **release:** preserve initial history and generated file ownership ([#48](https://github.com/japboy/lens/issues/48)) ([35424b4](https://github.com/japboy/lens/commit/35424b42f822ce7e09aea7d3eb7f0651f053c1f3))
* **target-selection:** accept the first inactive click ([#38](https://github.com/japboy/lens/issues/38)) ([3be7549](https://github.com/japboy/lens/commit/3be7549dbeacde986d07ff082ffbbe3d0455cd8c))
