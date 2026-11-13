BMM v0.1.1: a local-first, keyboard-driven mind mapper.

## Downloads

| Platform | Download |
| --- | --- |
| macOS 11+, Apple Silicon | `BMM-0.1.1-macos-arm64.zip` |
| macOS 11+, Intel | `BMM-0.1.1-macos-x86_64.zip` |
| Windows 10/11, x64 | `BMM-0.1.1-windows-x86_64.zip` |
| Debian 12+ / Ubuntu 22.04+, amd64 | `bmm_0.1.1_amd64.deb` |

**macOS:** unzip, move `BMM.app` to Applications, then open it. This build is
ad-hoc signed, **not Developer ID signed or notarized**. Gatekeeper may block it.
If you trust the download, follow Apple's Open Anyway option in System Settings
> Privacy & Security. Do not disable Gatekeeper globally.

**Windows:** extract the ZIP and open `BMM.exe`. It is a portable application,
not an installer. It is not Authenticode signed; SmartScreen may show a warning.

**Debian/Ubuntu:** install with `sudo apt install ./bmm_0.1.1_amd64.deb`.
Launch BMM from the application menu or run `bmm` in a graphical desktop session.
The package is built on Ubuntu 22.04 for its older glibc baseline and
install-checked on Debian 12, Ubuntu 22.04 and Ubuntu 24.04.

`SHA256SUMS.txt` contains checksums for all four downloads. On Linux, place the
downloads beside it and run `sha256sum --check --ignore-missing SHA256SUMS.txt`.
These checksums are not a publisher signature.

## Included

- Refined compact pill styling for the central root.
- Two-sided automatic layout, inline editing, folding and undo/redo.
- Multiple map tabs, underlying-file rename, native saves and local recovery.
- Multi-node dragging, rectangle selection and per-tab branch focus.
- Notes popup with F3, font controls, and an app-wide font size saved across restarts.
- F4 to focus the selected branch; F5 to restore the full map.
- Outline clipboard, Freeplane interchange and Mermaid export.

Notes use plain text; the popup no longer has formatting buttons or a preview.
Existing maps and app data retain their `.branchline` format and Branchline
storage location. Close existing instances before running this release.

## Verification and limitations

Native release tests run for both macOS architectures, Windows and Linux.
Packages are checked without launching a GUI. Native desktop, GPU, clipboard,
accessibility and IME behavior still need manual platform testing.

This early release is not Freeplane feature parity. Outline copy/paste does not
preserve notes, links or folding. A pre-existing macOS inline-label multiline
shortcut regression is not included in the targeted release test suite.
