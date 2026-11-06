"""Generate a tiny, offline Word revision fixture for browser review tests."""
from pathlib import Path
from zipfile import ZipFile
import base64
import hashlib
import json

root = Path(__file__).parent
date = '2026-11-05T00:00:00Z'


def revision(kind, identifier, text):
    tag = 'delText' if kind == 'del' else 't'
    return (f'<w:{kind} w:id="{identifier}" w:author="Wisp" w:date="{date}">'
            f'<w:r><w:{tag}>{text}</w:{tag}></w:r></w:{kind}>')


body = (
    '<w:p><w:r><w:t>Revision review fixture</w:t></w:r></w:p>'
    f'<w:p>{revision("del", 1, "Old conclusion")}'
    f'{revision("ins", 2, "Revised conclusion")}</w:p>'
    f'<w:p>{revision("del", 3, "Remove this sentence")}</w:p>'
    f'<w:p>{revision("ins", 4, "New supporting sentence")}</w:p>'
)
xml = ('<?xml version="1.0" encoding="UTF-8"?>'
       '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">'
       f'<w:body>{body}</w:body></w:document>')
with ZipFile(root / 'office-preview.docx') as source:
    with ZipFile(root / 'docx-review.docx', 'w') as output:
        for entry in source.infolist():
            output.writestr(entry, xml if entry.filename == 'word/document.xml'
                            else source.read(entry.filename))

# Both Swift and Rust decode this envelope; the backend test also checks it
# against the actual fixture bytes to catch parser/transport contract drift.
data = (root / 'docx-review.docx').read_bytes()
changes = []
for identifier, paragraph, kind, old, new in [
    (1, 'p2', 'rewrite', 'Old conclusion', 'Revised conclusion'),
    (3, 'p3', 'deletion', 'Remove this sentence', ''),
    (4, 'p4', 'insertion', '', 'New supporting sentence'),
]:
    tag = 'ins' if kind == 'insertion' else 'del'
    offset = xml.encode().index(f'<w:{tag} w:id="{identifier}"'.encode())
    changes.append(dict(id=f'change-{offset}', paragraph_id=paragraph, kind=kind,
                        author='Wisp', date=date, old=old, new=new))
contract = root.parent.parent / 'contracts/native-files/v1/docx-review.json'
contract.write_text(json.dumps(dict(
    schema='wisp.native-files.v1', project_id='project-1', session_id='session-1',
    path='/project/review.docx',
    review=dict(source_sha256=hashlib.sha256(data).hexdigest(), changes=changes,
                unsupported_count=0, writable=True),
    content=dict(path='/project/review.docx', mime='application/vnd.openxmlformats-officedocument.wordprocessingml.document',
                 text=None, base64=base64.b64encode(data).decode(), truncated=False, total_bytes=len(data)),
), ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
