# Originally written by mzxrules
from __future__ import annotations
from collections.abc import Sequence
import struct


class uint32:
    _struct = struct.Struct('>I')

    @staticmethod
    def write(buffer: bytearray, address: int, value: int) -> None:
        struct.pack_into('>I', buffer, address, value)

    @classmethod
    def read(cls, buffer: bytearray, address: int = 0) -> int:
        return cls._struct.unpack_from(buffer, address)[0]

    @staticmethod
    def bytes(value: int) -> bytearray:
        value = value & 0xFFFFFFFF
        return bytearray([(value >> 24) & 0xFF, (value >> 16) & 0xFF, (value >> 8) & 0xFF, value & 0xFF])

    @staticmethod
    def value(values: Sequence[int]) -> int:
        return ((values[0] & 0xFF) << 24) | ((values[1] & 0xFF) << 16) | ((values[2] & 0xFF) << 8) | (values[3] & 0xFF)
