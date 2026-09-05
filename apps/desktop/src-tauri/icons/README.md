# Lens icon resources

`icon.svg` is the sole authored mark: three filled paths on a 20-unit canvas. Its circular frame, clear inner boundary, and circle-cut L remain identical at every size. The clear areas are transparency, not white paint. Do not edit generated resources independently.

```sh
mise run generate:icons
mise run check:icons
```

The versioned `icon-family.json` contract selects explicit appearances, material, composition, filenames, and tray size. The generator validates the source, regenerates assets with the pinned Tauri CLI in temporary directories, and checks their contents. ICNS chunk order is normalized without changing payloads. Portable CI runs the same check; no additional image tool or system font is required.

| Resource                          | Role                                                                                   |
| --------------------------------- | -------------------------------------------------------------------------------------- |
| `icon-light.svg`, `icon-dark.svg` | Black or white flat mark, selected explicitly by its consumer                          |
| `icon-monochrome.svg`             | Opaque black plus alpha; source for the 36px / 18pt tray template                      |
| `icon-rich.svg`                   | Grayscale smoked-glass paint and clipped inner highlights; unchanged canonical paths   |
| `icon-macos.svg`, `icon.icns`     | Static macOS composition shared by the app header and the repository README            |
| `icon.ico`, desktop PNG files     | Transparent rich mark; prepared outputs, not Windows/Linux distribution support        |
| `icon-ios.svg`, `ios/`            | Rich mark rasterized with an opaque full-bleed background, without pre-rounded corners |
| `android/`                        | Separate rich foreground, opaque background, and undecorated monochrome layer          |

## Composition and appearance

The macOS composition places the rich mark on a neutral rounded plate, inset to 84% of the canvas. This scale is a Lens design choice, not a platform-mandated number. The plate is not part of the canonical mark. The broad tonal fill carries the material at small sizes; the restrained inner highlight cannot paint outside the mark or close the L. No blur, exterior shadow, or perspective is added.

The macOS 15.2+ application currently consumes only `icon.icns`. This is a static compatibility asset, not native Liquid Glass or automatic light/dark application-icon switching. The template tray has no material or background; AppKit owns tinting and Lens halves alpha when disabled. Explicit flat light/dark SVGs are available for other consumers, but external SVG images do not inherit a host page's `currentColor`.

Android layers use a 64dp circular mark inside a 108dp canvas, within the documented 66dp safe zone. The foreground and monochrome layer receive the same centered transform. Tauri CLI 2.11.4 applies `android_fg_scale` only to legacy output, so adaptive safe-zone scaling is encoded in the layer SVGs themselves. The Tauri manifest has one default input; a second isolated pass replaces only the macOS ICNS, leaving iOS corners unmasked and desktop marks transparent. A dedicated iOS pass rasterizes the full-bleed background inside `icon-ios.svg` before CLI compositing, avoiding 254/255 alpha rounding at antialiased edges; the source contains no rounded corner mask.

Layered Icon Composer packaging, native app-icon appearance variants, PWA manifests, and additional shipping platforms require separate integration and on-device verification. Do not import this already shaded static derivative as clean Icon Composer artwork.

## References

- [Apple app icon guidance](https://developer.apple.com/design/human-interface-guidelines/app-icons)
- [Preparing clean artwork for Icon Composer](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer)
- [AppKit template images](https://developer.apple.com/documentation/appkit/nsimage/istemplate)
- [Tauri icon generation](https://v2.tauri.app/develop/icons/) and [pinned CLI implementation](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-cli/src/icon.rs)
- [Android adaptive icon layers and safe zone](https://developer.android.com/develop/ui/compose/system/icon_design_adaptive)
- [GitHub repository-relative images](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax#images)

The repository header follows the compact centered app-icon pattern used by [IINA](https://github.com/iina/iina/blob/develop/README.md), rather than an inline heading icon as in [KeePassXC](https://github.com/keepassxreboot/keepassxc/blob/develop/README.md) or a banner as in [AltTab](https://github.com/lwouis/alt-tab-macos/blob/master/README.md). It reuses the generated SVG with a relative path, fixed dimensions, and alt text, and remains readable on light and dark GitHub backgrounds.
