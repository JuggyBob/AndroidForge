# Scoop manifest

This manifest targets the portable **v1.1.0** release asset. The `hash` field still carries the last
published (v1.0.0) value, `8066ea28cfe8344930841d285d14d8f835db69d50db497910375308bbbb20f01` for
`androidforge-desktop-windows-x86_64.zip`, and must be replaced from `dist\SHA256SUMS.txt` after the
v1.1.0 package is built. Validate the downloaded GitHub release asset locally before publishing the
manifest. This manifest is **not** submitted to or accepted by the official Scoop bucket. Do not
describe it as available from Scoop until a bucket maintainer publishes it.

<!-- replace after packaging: hash above is a v1.0.0 placeholder, not a v1.1.0 asset hash -->

Winget manifests were not published; the portable ZIP and Inno Setup installer are the supported
distribution formats for v1.1.0.
