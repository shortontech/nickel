# Smithay Example Attribution

The initial nested compositor plumbing in this crate is adapted from Smithay's
`smallvil` example at the v0.7.0 release. Smithay is distributed under the MIT
License and is copyright its contributors.

The native backend's connector and CRTC scanner follows the design of
`smithay-drm-extras` 0.1.0. That crate is distributed under the MIT License
and is copyright its contributors. Nickel carries the small scanner locally
to avoid coupling DRM discovery to the optional system `libdisplay-info` ABI.
