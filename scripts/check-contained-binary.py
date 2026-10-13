"""Check v2's input/cursor invariants and process containment in the PE imports."""
import pathlib
import struct
import sys


def imports(path):
    data = pathlib.Path(path).read_bytes()

    def u16(offset):
        return struct.unpack_from('<H', data, offset)[0]

    def u32(offset):
        return struct.unpack_from('<I', data, offset)[0]

    def string(offset):
        return data[offset:data.index(b'\0', offset)].decode('ascii')

    pe = u32(0x3C)
    assert data[:2] == b'MZ' and data[pe:pe + 4] == b'PE\0\0'
    optional = pe + 24
    assert u16(optional) == 0x20B, 'Expected x64 PE'
    sections = optional + u16(pe + 20)

    def offset(rva):
        for i in range(u16(pe + 6)):
            section = sections + i * 40
            size = max(u32(section + 8), u32(section + 16))
            address = u32(section + 12)
            if address <= rva < address + size:
                return u32(section + 20) + rva - address
        raise AssertionError(f'Unmapped RVA: {rva:x}')

    # Reject delay imports as well: they must not bypass this check.
    assert u32(optional + 112 + 13 * 8) == 0, 'Unexpected delay imports'
    descriptor = offset(u32(optional + 112 + 8))
    result = {}
    while u32(descriptor + 12):
        dll = string(offset(u32(descriptor + 12))).lower()
        thunk = offset(u32(descriptor) or u32(descriptor + 16))
        names = []
        while True:
            entry = struct.unpack_from('<Q', data, thunk)[0]
            if not entry:
                break
            assert not entry & (1 << 63), 'Unexpected ordinal import'
            names.append(string(offset(entry) + 2))
            thunk += 8
        # Rust raw-dylib linking may emit several descriptors for one DLL.
        result.setdefault(dll, []).extend(names)
        descriptor += 20
    return result


for binary in sys.argv[1:]:
    table = imports(binary)
    banned = {
        'SetWindowsHookExW', 'SetWindowsHookExA', 'SendInput', 'BlockInput',
        'ShowCursor', 'SetCursor', 'SetCursorPos', 'SetSystemCursor', 'ClipCursor',
        'SetCapture', 'DwmFlush', 'MagShowSystemCursor',
        'MagSetInputTransform', 'MagSetFullscreenTransform',
        'MagSetFullscreenColorEffect', 'SetWindowDisplayAffinity',
    }
    names = {name for group in table.values() for name in group}
    assert not names & banned, f'Unsafe imports: {names & banned}'
    required = {'CreateJobObjectW', 'AssignProcessToJobObject',
                'SetInformationJobObject', 'TerminateJobObject'}
    assert required <= names, f'Missing process containment imports: {required - names}'
    print(f'PASS: {binary}: no input interception/cursor mutation; process containment linked')

if len(sys.argv) == 1:
    raise SystemExit('Pass at least one Windows executable path')
