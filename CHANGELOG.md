# Changelog

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
