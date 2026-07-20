from machine import Pin, I2C

class TCA9535:
    i2c_address = 0x20

    def __init__(
        self,
        i2c: I2C,
    ) -> None:
        self._i2c = i2c

    def _read_reg(self, reg) -> int:
        """Read 8-bit register"""
        return self._i2c.readfrom_mem(self.i2c_address, reg, 1)[0]

    def _write_reg(self, reg, data) -> int:
        """Write 8-bit register"""
        self._i2c.writeto_mem(self.i2c_address, reg, bytes([data & 0xFF]))
