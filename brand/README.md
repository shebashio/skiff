# Skiff identity

The primary icon is `icon/skiff-master.png`, generated with the built-in
image-generation tool using the prompt below. It is retained in the repository;
normal builds do not invoke an image-generation service. The palette is midnight
navy, electric mint/teal, petrol blue, and icy white.

`python3 brand/generate.py brand` exports the vector logo family, favicon,
16–1024 px application PNGs, Windows ICO, and monochrome menu-bar template.
It requires `rsvg-convert`. The flat vector mark is an editable companion to
the dimensional application icon. Windows installer bitmap artwork is retained
in `installer/windows`; its PowerShell script can rebuild it from the new logos.

## Generation prompt

Use case: logo-brand. Create one exceptional production-ready desktop application icon for SKIFF, a fast, lightweight, independent container runtime and developer tool. No words or letters. A compact aggressive angular skiff / hydrofoil seen in a three-quarter isometric view, its sharply swept bow pointing upper right, formed from three interlocking geometric container-like facets. Bold instantly recognizable silhouette, tightly disciplined geometry, sophisticated engineered industrial design, not a cartoon boat. Electric mint/teal luminous top facets, deep petrol blue hull, a small icy white edge highlight. The mark sits centered on a near-black midnight navy rounded-square macOS app tile with restrained dimensional bevel and soft teal reflected light. Premium, confident, badass, clean. No skulls, sails, anchors, ocean scenery, ropes, decorative symbols, text, watermark, existing brand marks, or tiny details. A single icon only, straight-on orthographic view of the tile. 1024x1024 square composition; tile inset roughly 8% from each edge with genuinely transparent exterior, subtle compact shadow. Strong silhouette remains legible at 16px. Edge-to-edge icon artwork, no presentation board or mockup. Keep large clean shapes and high contrast; avoid excessive glow and generic neon cyberpunk clutter.
