"""Verify that a Windows x64 release does not require the VC++ redistributable."""
import argparse
from pathlib import Path
import struct


def check(path):
    data = path.read_bytes()
    if data[:2] != b'MZ':
        raise ValueError('Not a PE executable')
    pe = struct.unpack_from('<I', data, 60)[0]
    if data[pe:pe + 4] != b'PE\0\0' or struct.unpack_from('<H', data, pe + 4)[0] != 0x8664:
        raise ValueError('Release must target Windows x86_64')
    optional = pe + 24
    if struct.unpack_from('<H', data, optional)[0] != 0x20B:
        raise ValueError('Expected PE32+')
    count = struct.unpack_from('<H', data, pe + 6)[0]
    size = struct.unpack_from('<H', data, pe + 20)[0]
    sections = [struct.unpack_from('<IIII', data, optional + size + 40 * i + 8) for i in range(count)]

    def offset(rva):
        for virtual_size, address, raw_size, raw in sections:
            if address <= rva < address + max(virtual_size, raw_size):
                return raw + rva - address
        raise ValueError('Import address is outside PE sections')

    imports = struct.unpack_from('<I', data, optional + 112 + 8)[0]
    cursor = offset(imports)
    names = []
    while any(data[cursor:cursor + 20]):
        name_offset = offset(struct.unpack_from('<I', data, cursor + 12)[0])
        name = data[name_offset:data.index(0, name_offset)].decode('ascii')
        if name.lower().startswith(('vcruntime', 'msvcp', 'msvcr')):
            raise ValueError(f'Redistribution dependency: {name}. Build with -C target-feature=+crt-static')
        names.append(name)
        cursor += 20
    print(f'Windows x64 executable verified: {len(names)} DLL imports, no VC++ redistributable dependency')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    check(parser.parse_args().binary)
