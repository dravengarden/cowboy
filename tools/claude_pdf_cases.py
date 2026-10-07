"""PDF and file-type Read cases whose results must match native Claude Code.

Shared by the native-local baseline probe and the packaged remote acceptance.
Both sides need poppler's pdfinfo and pdftoppm where the file is read (the dev
shell provides them). `normalize` replaces the project root and reduces
base64 payloads to their length and digest.
"""
import hashlib
import re


def pdf(pages):
    """A minimal valid PDF with one line of text per page."""
    objects = ["<< /Type /Catalog /Pages 2 0 R >>", None]
    kids = []
    for index in range(pages):
        content = f"BT /F1 24 Tf 72 720 Td (Page {index + 1} marker) Tj ET"
        objects.append(f"<< /Length {len(content)} >>\nstream\n{content}\nendstream")
        objects.append("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
                       f"/Contents {len(objects)} 0 R /Resources << /Font << /F1 FONT >> >> >>")
        kids.append(len(objects))
    objects.append("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
    font = f"{len(objects)} 0 R"
    objects[1] = f"<< /Type /Pages /Kids [{' '.join(f'{kid} 0 R' for kid in kids)}] /Count {pages} >>"
    output, offsets = bytearray(b"%PDF-1.4\n"), []
    for number, body in enumerate(objects, 1):
        offsets.append(len(output))
        output += f"{number} 0 obj\n{body.replace('FONT', font)}\nendobj\n".encode()
    xref = len(output)
    output += f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode()
    output += b"".join(f"{offset:010d} 00000 n \n".encode() for offset in offsets)
    output += f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    return bytes(output)


def setup(directory):
    (directory / "pdf-one.pdf").write_bytes(pdf(1))
    (directory / "pdf-ten.pdf").write_bytes(pdf(10))
    (directory / "pdf-many.pdf").write_bytes(pdf(25))
    (directory / "pdf-upper.PDF").write_bytes(pdf(1))
    (directory / "pdf-empty.pdf").write_bytes(b"")
    (directory / "pdf-fake.pdf").write_text("not a pdf\n")
    (directory / "pdf-bytes.bin").write_bytes(pdf(1))
    (directory / "pdf-bytes-noext").write_bytes(pdf(1))
    (directory / "pdf-note.txt").write_text("hello\n")
    (directory / "pdf-archive.ZIP").write_text("text in a zip name\n")


CASES = [
    ("whole_one", {"file_path": "pdf-one.pdf"}),
    ("whole_ten", {"file_path": "pdf-ten.pdf"}),
    ("whole_many", {"file_path": "pdf-many.pdf"}),
    ("whole_upper_extension", {"file_path": "pdf-upper.PDF"}),
    ("whole_empty", {"file_path": "pdf-empty.pdf"}),
    ("whole_fake", {"file_path": "pdf-fake.pdf"}),
    ("pages_one", {"file_path": "pdf-one.pdf", "pages": "1"}),
    ("pages_range", {"file_path": "pdf-many.pdf", "pages": "2-3"}),
    ("pages_past_end", {"file_path": "pdf-many.pdf", "pages": "30"}),
    ("pages_range_past_end", {"file_path": "pdf-one.pdf", "pages": "1-3"}),
    ("pages_invalid", {"file_path": "pdf-one.pdf", "pages": "abc"}),
    ("pages_too_many", {"file_path": "pdf-many.pdf", "pages": "1-25"}),
    ("pages_open_range", {"file_path": "pdf-many.pdf", "pages": "24-"}),
    ("pages_fake", {"file_path": "pdf-fake.pdf", "pages": "1"}),
    ("pages_on_text", {"file_path": "pdf-note.txt", "pages": "1"}),
    ("pdf_bytes_binary_extension", {"file_path": "pdf-bytes.bin"}),
    ("pdf_bytes_no_extension", {"file_path": "pdf-bytes-noext"}),
    ("binary_extension_text", {"file_path": "pdf-archive.ZIP"}),
    ("binary_extension_missing", {"file_path": "pdf-missing.exe"}),
    ("missing_text", {"file_path": "pdf-missing.txt"}),
    ("missing_pdf", {"file_path": "pdf-missing.pdf"}),
]


def summarize(value):
    if isinstance(value, dict):
        return {key: (f"<{len(item)} base64 sha256:{hashlib.sha256(item.encode()).hexdigest()[:16]}>"
                      if key == "data" and isinstance(item, str) else summarize(item))
                for key, item in value.items()}
    if isinstance(value, list):
        return [summarize(item) for item in value]
    return value


def normalize(content, project):
    """Placement-independent form of one tool result's content."""
    def text(value):
        value = re.sub(r"\n*<system-reminder>.*?</system-reminder>\n*", "", value, flags=re.S)
        return value.replace(str(project), "<project>")
    if isinstance(content, str):
        return text(content)
    blocks = []
    for block in summarize(content):
        if block.get("type") == "text":
            if not text(block["text"]):
                continue
            block = {**block, "text": text(block["text"])}
        blocks.append(block)
    return blocks
