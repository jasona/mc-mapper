# Seed map assets

The reference shows seed `-7556262131344217808` in the Java 26.3 Overworld, centered at X 2839, Z −1418 with zoom 1.08. Keep the seed as a string in JavaScript to preserve all its digits.

| Local image | What it contains |
| --- | --- |
| [Map marker atlas](map/marker-atlas.webp) | Pixel art for structures, points of interest, and map markers. |
| [Feature picker atlas](ui/feature-atlas.webp) | The 22 × 22 icons for the feature picker. |
| [Button atlas](ui/button-atlas.png) and [icon mode toggle](ui/icon-mode-toggle.png) | Artwork for map controls. |
| [Page texture](ui/page-texture.png) and [content texture](ui/content-texture.png) | Background textures for the surrounding interface. |
| [Map view](reference/map-view.png) | The biome and feature layers combined into one image. |
| [Biome layer](reference/biomes.png) and [transparent feature layer](reference/features.png) | Two aligned captures of the same 1133 × 566 viewport. Place the feature layer over the biome layer at (0, 0). |

[manifest.json](manifest.json) records the view settings, image dimensions, and crop rectangles for the feature picker atlas.

The capture shows surface biomes. To pan to new coordinates or switch seeds, use a world generator or precomputed map data.
