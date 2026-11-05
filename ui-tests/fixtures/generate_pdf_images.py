"""Author the synthetic PDF image fixture; this does not run any app tests.

Requires Pillow with JPEG2000 and ImageCms support. All image data is synthetic.
The embedded RGB profile uses sRGB primaries with a linear (gamma 1) transfer
curve, so encoded gray 128 should display near sRGB 188, not unchanged at 128.
"""
from io import BytesIO
from pathlib import Path
import struct
import zlib

from PIL import Image, ImageCms, ImageDraw


def stream(data: bytes, fields: str = "") -> bytes:
    return f"<< /Length {len(data)} {fields} >>\nstream\n".encode() + data + b"\nendstream"


def linear_rgb_profile() -> bytes:
    profile = bytearray(ImageCms.ImageCmsProfile(ImageCms.createProfile("sRGB")).tobytes())
    # Normalize metadata to make repeated generation byte-stable.
    profile[24:36] = struct.pack(">6H", 2020, 1, 1, 0, 0, 0)
    profile[84:100] = bytes(16)
    count = struct.unpack_from(">I", profile, 128)[0]
    for index in range(count):
        position = 132 + 12 * index
        tag, offset, length = struct.unpack_from(">4sII", profile, position)
        if tag in (b"rTRC", b"gTRC", b"bTRC"):
            profile[offset:offset + length] = (b"para" + bytes(8) + struct.pack(">I", 65536)).ljust(length, b"\0")
    return bytes(profile)


colors = [(255, 0, 0), (0, 255, 0), (0, 0, 255), (0, 255, 255), (255, 0, 255), (255, 255, 0)]
image = Image.new("RGB", (96, 64))
draw = ImageDraw.Draw(image)
for index, color in enumerate(colors):
    x, y = index % 3 * 32, index // 3 * 32
    draw.rectangle((x, y, x + 31, y + 31), fill=color)
encoded = BytesIO()
image.save(encoded, format="JPEG2000", irreversible=False)
page_one = b"BT /F1 13 Tf 24 235 Td (JPEG2000: all six color blocks must be visible) Tj ET\nq 352 0 0 176 24 24 cm /Im1 Do Q"
page_two = b"BT /F1 13 Tf 24 235 Td (ICC linear RGB: the two gray blocks should match) Tj ET\nBT /F1 10 Tf 24 210 Td (Embedded ICC gray 128) Tj ET\nBT /F1 10 Tf 212 210 Td (Reference sRGB gray 188) Tj ET\nq 164 0 0 176 24 24 cm /Im2 Do Q\n0.737255 0.737255 0.737255 rg 212 24 164 176 re f"
objects = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 260] /Resources << /Font << /F1 7 0 R >> /XObject << /Im1 8 0 R >> >> /Contents 5 0 R >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 260] /Resources << /Font << /F1 7 0 R >> /XObject << /Im2 9 0 R >> >> /Contents 6 0 R >>",
    stream(page_one), stream(page_two),
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    stream(encoded.getvalue(), "/Type /XObject /Subtype /Image /Width 96 /Height 64 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /JPXDecode"),
    stream(zlib.compress(bytes([128, 128, 128]) * 16), "/Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace [/ICCBased 10 0 R] /BitsPerComponent 8 /Filter /FlateDecode"),
    stream(zlib.compress(linear_rgb_profile()), "/N 3 /Alternate /DeviceRGB /Filter /FlateDecode"),
]
pdf = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
offsets = []
for index, obj in enumerate(objects, 1):
    offsets.append(len(pdf))
    pdf.extend(f"{index} 0 obj\n".encode() + obj + b"\nendobj\n")
xref = len(pdf)
pdf.extend(f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode())
for offset in offsets:
    pdf.extend(f"{offset:010d} 00000 n \n".encode())
pdf.extend(f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
Path(__file__).with_name("pdf-images-jpx-icc.pdf").write_bytes(pdf)
