import time
from machine import Pin, I2C

class TLV320DAC3101:
    i2c_address = 0x18

    def __init__(
        self,
        i2c: I2C,
        pin_rst: Pin,
    ) -> None:
        self._i2c = i2c
        self._pin_rst = pin_rst

    def reset(self) -> None:
        self._pin_rst.value(0)
        time.sleep_us(1)
        self._pin_rst.value(1)
        time.sleep_ms(1)

    def _set_page(self, page) -> None:
        self._i2c.writeto_mem(self.i2c_address, 0, bytes([page & 0xFF]))

    def _read_reg(self, page, reg) -> int:
        """Read 8-bit register"""
        self._set_page(page)
        return self._i2c.readfrom_mem(self.i2c_address, reg, 1)[0]

    def _write_reg(self, page, reg, data) -> int:
        """Write 8-bit register"""
        self._set_page(page)
        self._i2c.writeto_mem(self.i2c_address, reg, bytes([data & 0xFF]))

    def setup(self) -> None:
        """
        Sets up the DAC with both channels muted, and speakers and headphones
        both disabled.

        # DAC Configuration

        ## Filter Selection
            PRB_P7 (interp. filter B) or PRB_P17 (interp. filter C) look like 
            basic, low power, stereo filters. They use IIR without any
            biquads.
            Section 6.3.10.1.4 talks about different interpolation filters.
            Type B is for up to 96kHz, Type C is specifically for 192 kHz.
            So we're going with *PRB_P7* for now.

        ## Clock dividers
            CODEC_CLKIN = NDAC × MDAC × DOSR × DAC_fS
            DAC_fS is 48KHz, and CODEC_CLKIN is MCLK, chosen to be 256 * DAC_fS

            So NDAC × MDAC × DOSR = 256

            For filter type B, DOSR must be a multiple of 4.
            (DOSR is "oversampling ratio"?)
            2.8 MHz < DOSR × DAC_fS < 6.2 MHz
            Thus, DOSR can be 64 or 128.

            DOSR = 128

            NDAC and MDAC can be from 1..128.
            NDAC should be as large as possible with "MDAC × DOSR / 32 ≥ RC",
            where RC for PRB_P7 is 6.

            MDAC = 2, NDAC = 1

            To increase NDAC, we can use the PLL to multiply MCLK.

        ## Common-mode voltage
            Based on the analog power supply. For Rev A, we have 3.3V.
            The options are 1.35 V, 1.5 V, 1.65 V, or 1.8 V, and it must be
            <= AVDD/2. 
            We'll go with 1.5V
        """

        # 1. Set up device.
        self.reset()

        # Do software reset?
        # self._write_reg(0,  0x01, 0x01)

        # 2. Program clock settings
        # PLL_clkin = MCLK, codec_clkin=MCLK
        self._write_reg(0, 0x04, 0x00)

        # PLL is unused
        # self._write_reg(0, 0x06, 0x08)
        # self._write_reg(0, 0x07, 0x00)
        # self._write_reg(0, 0x08, 0x00)
        # self._write_reg(0, 0x05, 0x91)

        # Program and power up NDAC ( = 1)
        self._write_reg(0, 0x0B, 0x81)

        # Program and power up MDAC ( = 2)
        self._write_reg(0, 0x0C, 0x82)

        # Program OSR
        #
        # DOSR = 128, DOSR(9:8) = 0, DOSR(7:0) = 128
        self._write_reg(0, 0x0D, 0x00)
        self._write_reg(0, 0x0E, 0x80)

        # Program codec interface (I2S, 16-bit, BCLK/WCLK inputs)
        self._write_reg(0, 0x1B, 0x00)

        # Program processing block. Select PRB_P7
        self._write_reg(0, 0x3C, 0x07)
        # Enable adaptive filtering
        self._write_reg(0, 0x00, 0x08)
        self._write_reg(0, 0x01, 0x04)
        self._write_reg(0, 0x00, 0x00)

        # DAC volume control through register, not pin
        self._write_reg(0, 0x74, 0x00)

        # 3. Program analog blocks

        # Program common-mode voltage (set to 1.5 V)
        self._write_reg(1, 0x1F, 0x0C)

        # Program headphone depop settings (power on = 800ms, step = 4ms)
        self._write_reg(1, 0x21, 0x4E)

        # Route DAC output to output amplifier mixer
        # LDAC to HPL, RDAC to HPR
        self._write_reg(1, 0x23, 0x44)

        # Unmute and set gain of output driver
        # Unmute HPL, set gain = 0 db
        self._write_reg(1, 0x28, 0x06)
        # Unmute HPR, set gain = 0 dB
        self._write_reg(1, 0x29, 0x06)
        # Unmute left speaker, set gain = 6 dB
        self._write_reg(1, 0x2A, 0x04)
        # Unmute right speaker, set gain = 6 dB
        self._write_reg(1, 0x2B, 0x04)

        # Configure output drivers
        # Enable HPL output analog volume, set = -9 dB
        self._write_reg(1, 0x24, 0x92)
        # Enable HPR output analog volume, set = -9 dB
        self._write_reg(1, 0x25, 0x92)
        # Enable speaker left output analog volume, set = -9 dB
        self._write_reg(1, 0x26, 0x92)
        # Enable speaker right output analog volume, set = -9 dB
        self._write_reg(1, 0x27, 0x92)

        # TODO: Apply waiting time determined by the de-pop settings and the soft-stepping settings
        #    of the driver gain or poll page 1 / register 63
        # ... 

        # 5. Power up DAC

        # Powerup DAC left and right channels (soft step enabled)
        self._write_reg(0, 0x3F, 0xD4)

        # Enable headphone detection
        self._write_reg(0, 0x43, 0x80)

    def set_volume(self, volume: int) -> None:
        """
        Sets DAC volume (range 0 to 255) for left and right.
        Mapped to DAC's volume range of -63.5 dB to 24 dB.
        """
        # map 0 -> -127, 255 -> 48
        # range = 175
        value = ((volume * 175) // 255) - 127
        self._write_reg(0, 0x41, value & 0xFF)
        self._write_reg(0, 0x42, value & 0xFF)

    def set_mute(self, mute: bool) -> None:
        # Left and right are individually controllable, but this sets them together.
        self._write_reg(0, 0x40, 0xC if mute else 0x0)

    def set_headphones_enabled(self, enabled: bool) -> None:
        if enabled:
            self._write_reg(1, 0x1F, 0xC2)
        else:
            self._write_reg(1, 0x1F, 0x02)

    def set_speakers_enabled(self, enabled: bool) -> None:
        if enabled:
            self._write_reg(1, 0x20, 0xC6)
        else:
            self._write_reg(1, 0x20, 0x06)

    def get_headphones_detected(self) -> bool:
        status = (self._read_reg(0, 0x43) >> 5) & 0b11
        return (status == 0b01) or (status == 0b11)