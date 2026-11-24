"""Independent check for the Antigravity location-gate signatures."""
import pathlib
import re
import struct
import tempfile


X64_ORIGINAL = re.compile(rb"\x80\x78\x08\x00\x74.\x48\x8b\x54\x24\x70\x48\x89\x50\x60", re.S)
X64_PATCHED = re.compile(rb"\xc6\x40\x08\x01\x90\x90\x48\x8b\x54\x24\x70\x48\x89\x50\x60", re.S)
X64_FIX = b"\xc6\x40\x08\x01\x90\x90"
ARM_ORIGINAL = re.compile(
    rb"\x03\x20\x40\x39[\x03\x23\x43\x63\x83\xa3\xc3\xe3].[\x00-\x07]\x36\xe3\x13\x48\xa9\x03\x10\x06\xa9",
    re.S,
)
ARM_PATCHED = re.compile(rb"\x23\x00\x80\x52\x03\x20\x00\x39\xe3\x13\x48\xa9\x03\x10\x06\xa9", re.S)
ARM_FIX = b"\x23\x00\x80\x52\x03\x20\x00\x39"
IDE_ORIGINAL = re.compile(rb"(resetIsTierGCPTos\(\),)this\.[A-Za-z_$0-9]+\.isGoogleInternal")
IDE_PATCHED = re.compile(rb"resetIsTierGCPTos\(\),true")


def pe(code: bytes) -> bytes:
    data = bytearray(0x200)
    data[0:2] = b"MZ"
    data[0x3C:0x40] = struct.pack("<I", 0x80)
    data[0x80:0x84] = b"PE\0\0"
    data[0x84:0x86] = struct.pack("<H", 0x8664)
    data[0x86:0x88] = struct.pack("<H", 1)
    data[0x94:0x96] = struct.pack("<H", 0x10)
    section = 0x80 + 24 + 0x10
    data[section + 16:section + 20] = struct.pack("<I", len(code))
    data[section + 20:section + 24] = struct.pack("<I", 0x200)
    data[section + 36:section + 40] = struct.pack("<I", 0x60000020)
    return bytes(data) + code


def executable_range(blob: bytes):
    pe_off = struct.unpack_from("<I", blob, 0x3C)[0]
    section = pe_off + 24 + struct.unpack_from("<H", blob, pe_off + 20)[0]
    raw_size, raw_off = struct.unpack_from("<II", blob, section + 16)
    return raw_off, raw_off + raw_size


def one(pattern, blob, span):
    hits = list(pattern.finditer(blob, span[0], span[1]))
    assert len(hits) == 1, hits
    return hits[0].start()


def check_binary():
    gate = b"\x80\x78\x08\x00\x74\x12\x48\x8b\x54\x24\x70\x48\x89\x50\x60"
    blob = pe(b"\x00" * 16 + gate)
    span = executable_range(blob)
    off = one(X64_ORIGINAL, blob, span)
    assert off == 0x210
    assert not X64_PATCHED.search(blob, *span)
    patched = bytearray(blob)
    patched[off:off + len(X64_FIX)] = X64_FIX
    assert one(X64_PATCHED, bytes(patched), span) == off
    assert not X64_ORIGINAL.search(bytes(patched), *span)

    arm = bytes([0x03, 0x20, 0x40, 0x39, 0x43, 0x02, 0x03, 0x36, 0xE3, 0x13, 0x48, 0xA9, 0x03, 0x10, 0x06, 0xA9])
    assert ARM_ORIGINAL.fullmatch(arm)
    forced = bytearray(arm)
    forced[: len(ARM_FIX)] = ARM_FIX
    assert ARM_PATCHED.fullmatch(bytes(forced))


def check_files():
    with tempfile.TemporaryDirectory() as raw:
        root = pathlib.Path(raw)
        (root / "resources" / "bin").mkdir(parents=True)
        (root / "resources" / "app" / "out").mkdir(parents=True)
        server = root / "resources" / "bin" / "language_server.exe"
        gate = b"\x80\x78\x08\x00\x74\x21\x48\x8b\x54\x24\x70\x48\x89\x50\x60"
        server.write_bytes(pe(gate))
        span = executable_range(server.read_bytes())
        off = one(X64_ORIGINAL, server.read_bytes(), span)
        patched = bytearray(server.read_bytes())
        patched[off:off + len(X64_FIX)] = X64_FIX
        server.with_suffix(".exe.larbak").write_bytes(server.read_bytes())
        server.write_bytes(patched)
        assert one(X64_PATCHED, server.read_bytes(), span) == off
        server.write_bytes(server.with_suffix(".exe.larbak").read_bytes())
        assert one(X64_ORIGINAL, server.read_bytes(), span) == off

        main = root / "resources" / "app" / "out" / "main.js"
        original = b"keep resetIsTierGCPTos(),this.ab.isGoogleInternal tail"
        main.write_bytes(original)
        replaced, count = IDE_ORIGINAL.subn(rb"\1true", original, count=1)
        assert count == 1
        assert replaced == b"keep resetIsTierGCPTos(),true tail"
        assert IDE_PATCHED.search(replaced)


if __name__ == "__main__":
    check_binary()
    check_files()
    print("eligibility signature check passed")
