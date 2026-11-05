# Native UI fonts

`inter-latin.ttf` and `jetbrains-mono-400.ttf` are lossless OpenType decompressions
of the corresponding checked-in `ui/fonts/*.woff2` files. They keep the original
family names and outlines. WinUI loads these app-local assets without installing
fonts on the user's machine. The Latin UI subset falls back to Microsoft YaHei UI
for Chinese, matching the Windows portion of the WebView font stack.

`wisp-serif.ttf` uses the outlines of the checked-in `source-serif-4-latin.woff2`
for the welcome heading, matching WebView's serif typography. Its internal family
and PostScript names are renamed to Wisp Serif for the Source reserved-name terms;
the glyphs, weights and metrics are unchanged. It falls back to a Chinese serif.

These fonts use the SIL Open Font License. The upstream licenses are included:
- https://github.com/rsms/inter/blob/master/LICENSE.txt
- https://github.com/JetBrains/JetBrainsMono/blob/master/OFL.txt
- https://github.com/adobe-fonts/source-serif/blob/release/LICENSE.md

To regenerate after updating the WebView assets, with FontTools and Brotli:

```python
from pathlib import Path
from fontTools.ttLib import TTFont

output = Path("apps/windows/Wisp.Science.Preview/Assets/Fonts")
for name in ("inter-latin", "jetbrains-mono-400"):
    font = TTFont(f"ui/fonts/{name}.woff2", recalcTimestamp=False)
    font.flavor = None
    font.save(output / f"{name}.ttf")

font = TTFont("ui/fonts/source-serif-4-latin.woff2", recalcTimestamp=False)
for record in font["name"].names:
    if record.nameID in (1, 3, 4, 6, 16, 21, 25):
        name = record.toUnicode().replace("Source Serif 4", "Wisp Serif").replace("SourceSerif4", "WispSerif")
        record.string = name.encode(record.getEncoding())
font.flavor = None
font.save(output / "wisp-serif.ttf")
```
