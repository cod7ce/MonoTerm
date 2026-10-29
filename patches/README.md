# Maintained terminal patch

`gpui-ghostty.patch` is applied to `cod7ce/gpui-ghostty` commit
`ff9859b3ea0888c2469bdc0cafeddd1076a2e498` by `scripts/prepare-vendor.py`.
It contains MonoTerm's complete local terminal integration: the Ghostty 1.4 C
adapter and PNG decoder, image rendering/animation, VT text and reply handling,
font/input/selection behavior, and regression tests. The patch also points GPUI
to the shared Zed checkout. Upstream licenses remain in the checkout and bundle.

Ghostty itself is separately pinned to
`83fb95624b0439ee660c2679b43b38c8b63769cd` (v1.4.0) by bootstrap. The patch never
changes upstream Ghostty source. Zed is pinned in the same bootstrap script.
Herdr's adapted client libraries are source-vendored in `vendor/herdr` with
license and provenance notices.

Preparation is idempotent: an exact reverse check recognizes an already applied
patch; an exact forward check permits applying it to the baseline. A divergent
local checkout is left untouched and reported. Do not reset or discard local
changes to make bootstrap pass.

After changing the maintained terminal code, regenerate this patch from the
baseline, including newly added source files, and verify forward application in
a separate baseline checkout plus reverse validation in the working checkout.
Run application and terminal regressions using MonoTerm's root Cargo.lock.
