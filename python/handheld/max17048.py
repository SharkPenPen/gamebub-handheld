import struct
from machine import I2C

class MAX17048:
    i2c_address = 0x36

    def __init__(
        self,
        i2c: I2C,
    ) -> None:
        self._i2c = i2c

    def _read_reg(self, reg: int) -> int:
        """Reads a 16-bit big-endian register."""
        raw = self._i2c.readfrom_mem(self.i2c_address, reg, 2)
        return struct.unpack('>H', raw)[0]

    def ping(self) -> bool:
        """
        Returns whether the device is active and responding.

        May give false positives (can be powered by I2C bus sometimes).
        """
        try:
            chip_version = self._read_reg(0x08)
            return (chip_version & 0xFFF0) == 0x0010
        except:
            return False

    def battery_voltage(self) -> float:
        """Returns the battery voltage in volts."""
        return (self._read_reg(0x2) * 78.125) / 1_000_000

    def battery_level(self) -> float:
        """Returns the battery state of charge, in percent."""
        return self._read_reg(0x4) / 256

    def charge_rate(self) -> float:
        """Returns the charge or discharge rate of the battery, in %/hr"""
        return self._read_reg(0x16) * 0.208
