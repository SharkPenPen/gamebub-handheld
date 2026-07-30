import machine
from machine import SPI, I2C, Pin
import time
import random
import framebuf

from .ili9488 import ILI9488
from .max17048 import MAX17048
from .tlv320dac3101 import TLV320DAC3101
from .fpga import FPGA
from .tca9535 import TCA9535

pin_btn_home = Pin(0, Pin.IN)
pin_power_switch_n = Pin(1, Pin.IN)  # in/out
mcu_irq = Pin(2, Pin.IN)
status_led = Pin(3, Pin.OUT)
pin_btn_vol_up = Pin(4, Pin.IN)
pin_btn_vol_down = Pin(5, Pin.IN)
lcd_backlight = Pin(6, Pin.OUT)
lcd_reset = Pin(7, Pin.OUT)
lcd_cs = Pin(15, Pin.OUT)
lcd_dc = Pin(16, Pin.OUT)
dac_reset = Pin(40, Pin.OUT)
fpga_power = Pin(46, Pin.OUT)
fpga_done = Pin(17, Pin.IN)
fpga_program_b = Pin(18, Pin.OPEN_DRAIN, value=1)
fpga_init_b = Pin(8, Pin.IN)
fpga_spi_cs = Pin(10, Pin.OUT)
pin_vbus_pgood_n = Pin(41, Pin.IN)
pin_batt_chg = Pin(42, Pin.IN)
pin_sd_detect = Pin(37, Pin.IN)

pin_btn_home.irq(handler = lambda _: print("IRQ: home"))
pin_power_switch_n.irq(handler = lambda _: print("IRQ: power"))

# BUG (Rev A): FPGA can interfere with LCD SPI, also using LCD SPI while FPGA is unpowered probably isn't great
fpga_power.value(1)

i2c = I2C(0, sda=Pin(38), scl=Pin(39), freq=400_000)

class SpiGetter(object):
    """SPI bus 'getter' to allow changing baudrate"""
    def __init__(self) -> None:
        self._spi = None
        self._baudrate = None

    def __call__(self, baudrate: int) -> SPI:
        if self._spi is None or self._baudrate != baudrate:
            self._baudrate = baudrate
            # polarity = 0, phase = 0, firstbit = MSB
            self._spi = SPI(2, baudrate=baudrate, sck=Pin(12), mosi=Pin(11), miso=Pin(13))
        return self._spi
spi = SpiGetter()

lcd_pwm = machine.PWM(lcd_backlight, freq=30_000, duty=256)

fuel_gauge = MAX17048(i2c)

# P0: [R, X, Y, A, B, nHDMI_HPD, n/a, n/a]
# P1: [n/a, L, Up, Right, Left, Down, Select, Start]
io_expander = TCA9535(i2c)
io_expander._read_reg(0)
io_expander._read_reg(1)

# On IMU, change IRQ to active-low open-drain.
i2c.writeto_mem(0x6A, 0x12, b"\x00\x10")

print("Initializing LCD")
lcd = ILI9488(lambda: spi(10_000_000), pin_rst=lcd_reset, pin_cs=lcd_cs, pin_dc=lcd_dc)
lcd.setup()

print("Initializing DAC")
dac = TLV320DAC3101(i2c, dac_reset)
dac.setup()
dac.set_volume(100)
# dac.set_mute(True)
dac.set_mute(False)
dac.set_headphones_enabled(True)
dac.set_speakers_enabled(False)

audio_muted = True
def volume_btn_handler(pin):
    # TODO improve
    global dac
    global audio_muted
    if pin is pin_btn_vol_down:
        print("IRQ: volume down", pin_btn_vol_down.value())
        if pin_btn_vol_down.value() == 0:
            dac.set_speakers_enabled(audio_muted)
            audio_muted = not audio_muted
            print("Toggling speaker mute: ", audio_muted)
    if pin is pin_btn_vol_up:
        print("IRQ: volume up", pin_btn_vol_up.value())

pin_btn_vol_up.irq(handler = volume_btn_handler)
pin_btn_vol_down.irq(handler = volume_btn_handler)

print("Initializing MCU IRQ")
def irq_handler(_pin):
    print("IRQ fall at time(ms) ", (time.time_ns() // 1_000_000))

    # FPGA irqs. Read and ack.
    fpga_irq = fpga.spi_read_u32(0x10)
    fpga.spi_write_u32(0x10, 0xFFFFFFFF)
    print("  FPGA: ", hex(fpga_irq))

    # I/O expander
    io_0 = io_expander._read_reg(0)
    io_1 = io_expander._read_reg(1)
    print("  I/O expander: ", hex(io_0), hex(io_1))

    if mcu_irq.value() == 0:
        print("ERROR: nIRQ is still low!")

    # TODO others...
mcu_irq.irq(handler = irq_handler, trigger = Pin.IRQ_FALLING)

print("Passing control of display to FPGA")
# BPGRAM=0 (write to mem), RM=1 (RGB interface), DM=1 (DOTCLK), RCM=0 (DE mode), 
# lcd._write_cmd(0xB6, bytes([0x32]))
# hsync, vsync, enable polarity high, dotclock: sample on falling edge
lcd._write_cmd(0xB0, bytes([0x0E]))
# TESTING: bypass memory, direct to shift register:
lcd._write_cmd(0xB6, bytes([0xB2, 0x62]))   # BYPASS memory, direct to shift register... and rotate gate/drive 
lcd._write_cmd(0xB4, bytes([0x00]))  # set display inversion to "column inversion"

print("Programming FPGA")
fpga = FPGA(
    fpga_power,
    fpga_done,
    fpga_program_b,
    fpga_init_b,
    fpga_spi_cs,
    program_spi = lambda: spi(80_000_000),
    fpga_spi = lambda: spi(20_000_000),
)
fpga.program()
# Set FPGA submodule to running and out of reset.
fpga.spi_write_u32(0x0000_0000, 0b11)


# Overlay Framebuffer
framebuf_storage = bytearray(240 * 160 * 2)
fb = framebuf.FrameBuffer(framebuf_storage, 240, 160, framebuf.RGB565)
fb.fill(0x7FFF) # full white
def update_overlay():
    fpga.spi_write(26, 0x38000000, framebuf_storage)
    fpga.show_overlay()


# # 1-bit SPI
# sd_card = machine.SDCard(
#     slot=3, # SPI
#     width=1,
#     cd = Pin(37),
#     sck = Pin(45),
#     miso = Pin(35),
#     mosi = Pin(48),
#     cs = Pin(47),
#     freq = 20_000_000,
# )
# # 4-bit SDIO (requires micropython changes to set pins)
# sd_card = machine.SDCard(
#     slot=1, # SDIO
#     width=4,
#     freq=40_000_000,
# )
# os.mount(sd_card, "/sd")
# s = time.time_ns() ; print(len(open('/sd/BOOT.bin', 'rb').read(256 * 1024))) ; e = time.time_ns()
# print("time (ns)", e - s)




# INIT CODE FROM EXAMPLE

### END

# SRAM test
def do_sram_test(max_address = 2**4, seed = 1234):
    random.seed(seed)
    for i in range(0, max_address):
        addr = 0x8000_0000 | i
        data = bytes(random.getrandbits(8) for _ in range(2))
        fpga.spi_write(addr, data)

    random.seed(seed)
    for i in range(0, max_address):
        addr = 0x8000_0000 | i
        data = bytes(random.getrandbits(8) for _ in range(2))
        actual = fpga.spi_read(addr, 2)
        if data != actual:
            print(f"mismatch at addr={i}, actual={actual}, expected={data}")


def do_sdram_test(max_address = 1024, seed = 1234):
    start_time = time.time_ns()
    random.seed(seed)
    chunk_size = 1024
    for address in range(0, max_address, chunk_size):
        data = bytes(random.getrandbits(8) for _ in range(chunk_size))
        fpga.sdram_write(address, data)
    duration = time.time_ns() - start_time
    print(f"writing time(s)={duration / 1_000_000_000}")
    start_time = time.time_ns()
    random.seed(seed)
    for address in range(0, max_address, chunk_size):
        expected = bytes(random.getrandbits(8) for _ in range(chunk_size))
        actual = fpga.sdram_read(address, chunk_size)
        if expected != actual:
            print(f"mismatch at addr={address}, actual={actual}, expected={expected}")
    duration = time.time_ns() - start_time
    print(f"reading time(s)={duration / 1_000_000_000}")


class RomHeader:
    def __init__(self, rom_data: bytes) -> None:
        self.mbc = 0
        self.has_ram = False
        self.has_rtc = False
        self.has_rumble = False

        emu_configs = {
            0x00: dict(mbc=0),
            0x01: dict(mbc=1),
            0x02: dict(mbc=1, has_ram=True),
            0x03: dict(mbc=1, has_ram=True),
            0x05: dict(mbc=2, has_ram=True),
            0x06: dict(mbc=2, has_ram=True),
            0x0F: dict(mbc=3, has_rtc=True),
            0x10: dict(mbc=3, has_ram=True, has_rtc=True),
            0x11: dict(mbc=3),
            0x12: dict(mbc=3, has_ram=True),
            0x13: dict(mbc=3, has_ram=True),
            0x19: dict(mbc=4),
            0x1C: dict(mbc=4, has_rumble=True),
            0x1A: dict(mbc=4, has_ram=True),
            0x1B: dict(mbc=4, has_ram=True),
            0x1D: dict(mbc=4, has_ram=True, has_rumble=True),
            0x1E: dict(mbc=4, has_ram=True, has_rumble=True),
        }
        self.cartridge_type = rom_data[0x147]
        if self.cartridge_type not in emu_configs:
            raise RuntimeException(f"Unsupported cart {hex(self.cartridge_type)}")
        config = emu_configs[self.cartridge_type]
        self.mbc = config['mbc']
        self.has_ram = config.get('has_ram', False)
        self.has_rtc = config.get('has_rtc', False)
        self.has_rumble = config.get('has_rumble', False)

        self.rom_size = 32 * 1024 * (1 << rom_data[0x148])
        self.ram_size = {0: 0, 2: (8 * 1024), 3: (32 * 1024), 4: (128 * 1024), 5: (64 * 1024)}[rom_data[0x149]]
        if self.mbc == 2:
            self.ram_size = 512

    def get_emu_cart_config(self) -> int:
        value = 1  # Lowest bit: is emulated cartridge enabled
        value |= self.mbc << 1
        value |= int(self.has_ram) << 4
        value |= int(self.has_rtc) << 5
        value |= int(self.has_rumble) << 6
        return value

def load_rom(path = '/Tetris.gb', chunk_size=1024):
    # bit 0: pause, bit 1: reset
    fpga.spi_write_u32(0x0000_0000, 0b00)

    rom_header = None
    print("Transferring ROM...")
    rom_size = 0
    start_time = time.time_ns()
    with open(path, 'rb') as f:
        rom_header = RomHeader(f.read(512))
        f.seek(0)

        while True:
            data = f.read(chunk_size)
            if len(data) == 0:
                break
            fpga.sdram_write(rom_size, data)
            rom_size += len(data)

    duration = time.time_ns() - start_time
    print(f"Done. len={rom_size} time(s)={duration / 1_000_000_000}")

    ram_address_start = 0b110_0000_0000_0000_0000
    ram_size = 0
    try:
        ram_path = None
        if path[-3:] == ".gb":
            ram_path = path[:-3] + ".sav"
        elif path[-4:] == ".gbc":
            ram_path = path[:-4] + ".sav"
        with open(ram_path, 'rb') as f:
            print("Loading RAM.")
            chunk_size = 256
            address = ram_address_start
            while True:
                data = f.read(chunk_size)
                if len(data) == 0:
                    break
                fpga.sram_write(address, data)
                address += len(data)
                ram_size += len(data)
            print(f"Done. len={ram_size}")
    except:
        print("Not loading RAM.")
        # Clear SRAM to 0s.
        for i in range(ram_address_start, ram_address_start + rom_header.ram_size, 1024):
            fpga.sram_write(i, b"\xFF" * 1024)

    # take out of reset, leave paused
    fpga.spi_write_u32(0x0000_0000, 0b10)

    # configure emu cart
    config = rom_header.get_emu_cart_config()
    fpga.spi_write_u32(0xC000_0000, config)
    fpga.spi_write_u32(0xC000_0004, 0)  # rom address
    fpga.spi_write_u32(0xC000_0008, rom_header.rom_size - 1)  # rom mask
    fpga.spi_write_u32(0xC000_000C, ram_address_start)  # ram address
    fpga.spi_write_u32(0xC000_0010, rom_header.ram_size - 1)  # ram mask

    # then go...
    fpga.spi_write_u32(0x0000_0000, 0b11)

    return rom_header

"""

config = handheld.load_fpga_sram('/Metroid_II.gb')
"""



## write frame data?

# def lcd_write_data_pix(DH, DL):
#     LD = DH<<8
#     LD |= DL

#     R1=(0x1f&(LD>>11))*2
#     R1<<=2
#     G1=0x3f&(LD>>5)
#     G1<<=2
#     B1= (0x1f&LD)*2
#     B1<<=2

#     lcd_cs.value(0)
#     lcd_dc.value(1) # data
#     print([R1, G1, B1])
#     display_spi.write(bytes([R1, G1, B1])) 
#     lcd_cs.value(1)

# def lcd_write_data_u16(y):
#     m=y>>8
#     n=y
#     lcd_write_data_pix(m,n)


# lcd.set_pos(0,319,0,479)
# for i in range(0, 320):
#     for j in range(0, 480):
#         lcd_write_data_u16(0xf800)

# print("Writing screen...")
# for i in range(0, 256):
#     data = bytes([248, i, 0] * 480)
#     lcd.set_pos(0,319,0,479)
#     lcd_cs.value(0)
#     lcd_dc.value(1)
#     for i in range(0, 320):
#         display_spi.write(data)
#     lcd_cs.value(1)
# print("Done writing screen")

# lcd.set_pos(0,479,0,319)
# for y in range(0, 320):
#     for x in range(0, 480):
#         data = bytes([x & 0xFF, (y << 3) & 0xFF, 0])
#         lcd_cs.value(0)
#         lcd_dc.value(1)
#         display_spi.write(data)
#         lcd_cs.value(1)
#         # time.sleep_us(1000)
