# Synthetic PDF image fixture

`pdf-images-jpx-icc.pdf` contains two synthetic pages, with no user data:

1. A lossless JPEG2000 (`JPXDecode`) image with red, green, blue, cyan, magenta
   and yellow blocks. All six blocks must appear.
2. An image encoded as RGB gray 128 using an embedded linear RGB ICC profile.
   Its displayed color should be approximately sRGB gray 188, matching the
   reference block on the right. Falling back to DeviceRGB makes the left block
   too dark and must fail the regression check.

The Playwright source verifies actual canvas colors and local OpenJPEG/qcms
resource requests, rather than accepting a nonblank page as success.

Regenerate with `python ui-tests/fixtures/generate_pdf_images.py` from the repo
root. This authoring script requires Pillow with JPEG2000 and ImageCms support;
CI reads the checked-in PDF and does not need Pillow. Regeneration does not run
tests or launch an application.
