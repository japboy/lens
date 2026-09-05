# Install Lens

Lens is currently distributed to testers who have read access to this private repository.
The app requires **Apple Silicon and macOS 15.2 or later**.

The application uses an **ad-hoc code signature**. The DMG is unsigned, and the
application and DMG have not been notarized by Apple. An ad-hoc signature checks
code integrity; it does not identify an Apple-verified developer.

1. Sign in to GitHub and open [Releases](https://github.com/japboy/lens/releases).
   Download `Lens_<version>_aarch64.dmg` and `SHA256SUMS` from the same release.
   GitHub's automatic Source code archives are not application installers.
2. In Terminal, change to the directory containing those two files and run:
   ```sh
   shasum -a 256 -c SHA256SUMS
   ```
   Continue only when the DMG reports `OK`. A mismatch requires a fresh download
   or a report to the maintainer.
3. Open the DMG. Drag `Lens.app` onto Applications, then eject the disk image.
   Launch the installed copy from Applications.
4. macOS may block the first launch. After attempting to launch the installed
   copy, open **System Settings > Privacy & Security > Open Anyway** and confirm.
   Follow [Apple's current instructions](https://support.apple.com/en-us/102445).
   Right-clicking Open alone is not guaranteed to work.
5. Follow Lens's prompts to grant Accessibility and Screen Recording access.
   Reopen the application if macOS requests it.
6. Select an Agent in Settings. Lens downloads its managed runtime separately.
   Complete the provider's authentication flow and use any required provider
   subscription or account. Credentials are not included in the DMG.

If macOS reports that the app is damaged or Open Anyway is unavailable, stop and
report the exact macOS version and message to the maintainer. Do not disable
Gatekeeper globally or re-sign the app. A quarantine-removal workaround is not
part of these instructions until it has been reproduced and validated for Lens.

## Update or remove

Quit Lens before replacing the existing `/Applications/Lens.app` with a newer
verified download. Updates are manual; the application has no automatic updater.
The replacement may need Accessibility or Screen Recording access to be granted
again in System Settings. If permission appears granted but the app cannot use
it, remove the old Lens permission entry, add the installed copy again, and
restart Lens. A constant bundle identifier does not guarantee permission
continuity with ad-hoc signatures; see
[Apple's code-signing requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements).

To remove the application, quit it and move `/Applications/Lens.app` to Trash.
This does not remove your saved settings, provider credentials or separately
downloaded Agent runtimes.

## Tester expectations

The initial release is for functional testing. Source extraction and monitoring
coverage depend on the selected application's Accessibility support. Report
the Lens version, macOS version, affected operation and a minimal reproduction.
Avoid including provider tokens, credentials or captured private source content.
