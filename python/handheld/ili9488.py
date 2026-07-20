import time
from machine import SPI, Pin

class ILI9488:
    def __init__(
        self,
        spi: "Callable[[], SPI]",
        pin_rst: Pin,
        pin_cs: Pin,
        pin_dc: Pin
    ) -> None:
        self._spi = spi
        self._pin_rst = pin_rst
        self._pin_cs = pin_cs
        self._pin_dc = pin_dc

    def _write_cmd(self, cmd: int, params: bytes = bytes()) -> None:
        self._spi()  # take spi bus
        self._pin_cs.value(0)
        self._pin_dc.value(0) # command
        self._spi().write(bytes([cmd]))
        if params:
            self._pin_cs.value(1) # Is toggling cs necessary?
            self._pin_cs.value(0) 
            self._pin_dc.value(1) # data
            self._spi().write(params)
        self._pin_cs.value(1)

    def _read_cmd(self, cmd, len=2):
        self._spi()  # take spi bus
        self._pin_cs.value(0)
        self._pin_dc.value(0) # command
        self._spi().write(bytes([cmd]))
        #self._pin_cs.value(1) # Is toggling cs necessary?
        #self._pin_cs.value(0) 
        #self._pin_dc.value(1) # data
        data = self._spi().read(len)
        self._pin_cs.value(1)
        return data

    def setup(self) -> None:
        self._pin_cs.value(1)

        self._pin_rst.value(0)
        time.sleep_us(100)
        self._pin_rst.value(1)
        time.sleep_ms(120)
        # Must wait 120ms to send "SLEEP OUT"

        # "Adjust Control 3": params have no specified meaning
        self._write_cmd(0xF7, bytes([0xA9, 0x51, 0x2C, 0x82]))

        # MADCTL (Memory Access Control):
        # BGR order, rotate display orientation
        # (vendor provided is 0x48)
        # TODO might want to also update LCD shift register direction somewhere?
        # self._write_cmd(0x36, bytes([0xE8])) # Rotated
        self._write_cmd(0x36, bytes([0xC8])) # original  (works with *native* bitstream, W=320,H=480)

        # Interface Pixel Format: DPI = 18 bit, DBI = 18 bit
        #  (16 bit doesn't work)
        self._write_cmd(0x3A, bytes([0x66]))

        # Interface Mode Control: use separate SPI read/write wires
        # TODO: THIS SHOULD BE 0b1000_0000 -- use the same SDA wire,
        #    because ILI9488 does *NOT* tri-state SDO when CS is high, 
        #    meaning it screws up the SPI bus. It should be *disconnected*
        #    (or in a board revision, a tri-state buffer added.)
        self._write_cmd(0xB0, bytes([0x00]))

        # Display Inversion Control: 2-dot (from vendor)
        self._write_cmd(0xB4, bytes([0x02]))

        # Frame rate: 60 Hz
        self._write_cmd(0xB1, bytes([0xA0, 0x11]))

        # Power Control 1: Vreg1out=4.56  Vreg2out=-4.56 (from vendor)
        self._write_cmd(0xC0, bytes([0x0f, 0x0f]))

        # Power Control 2: VGH=15.81 ,VGL=-10.41,DDVDH=5.35,DDVDL=-5.23  VCL=-2.7 (from vendor)
        self._write_cmd(0xC1, bytes([0x41]))

        # Power Control 3: (from vendor)
        self._write_cmd(0xC2, bytes([0x22]))

        # VCOM Control (from vendor)
        self._write_cmd(0xC5, bytes([0x00, 0x53, 0x80]))

        # Entry Mode Set (from vendor)
        self._write_cmd(0xB7, bytes([0xC6]))

        # Positive Gamma Control (from vendor)
        self._write_cmd(0xE0, bytes([0x00, 0x08, 0x0C, 0x02, 0x0E, 0x04, 0x30, 0x45, 0x47, 0x04, 0x0C, 0x0A, 0x2E, 0x34, 0x0F]))

        # Negative Gamma Control (from vendor)
        self._write_cmd(0xE1, bytes([0x00, 0x11, 0x0D, 0x01, 0x0F, 0x05, 0x39, 0x36, 0x51, 0x06, 0x0F, 0x0D, 0x33, 0x37, 0x0F]))

        # Display Inversion ON (?? from vendor)
        self._write_cmd(0x21)

        # Sleep out
        self._write_cmd(0x11)
        time.sleep_ms(10)

        # Display on
        self._write_cmd(0x29)
        time.sleep_ms(10)

    # TODO remove or change
    def set_pos(self, xs, xe, ys, ye):
        # Column Address Set
        self._write_cmd(0x2A, bytes([xs>>8, xs&0xff, xe>>8, xe&0xff]))
        # Page Address Set
        self._write_cmd(0x2B, bytes([ys>>8, ys&0xff, ye>>8, ye&0xff]))
        # Begin Memory Write
        self._write_cmd(0x2C)