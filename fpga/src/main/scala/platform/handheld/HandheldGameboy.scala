package platform.handheld

import chisel3._
import chisel3.util._
import gameboy.Gameboy
import gameboy.cart.emu.{EmuCartConfig, EmuCartridge, Mbc3RtcAccess, RtcState}
import lib.mem.{MemoryInterface, MemoryMap, RegisterMap}

object HandheldGameboy {
  class Config extends Bundle {
    val isCgb = Bool()
  }
}

/**
 * Clocked by the 8.3886 MHz "Gameboy" clock.
 */
class HandheldGameboy extends Module with HandheldModule {
  val io = IO(new HandheldIo)
  def framebufferW = 160
  def framebufferH = 144
  def clockSystemHz = 8 * 1024 * 1024
  def clockSdramHz = clockSystemHz * 4

  // Config
  val configRegSystem = RegInit(0.U.asTypeOf(new HandheldGameboy.Config))
  val configRegEmuCart = RegInit(0.U.asTypeOf(new EmuCartConfig))
  val configRegRomAddress = RegInit(0.U(19.W))
  val configRegRomMask = RegInit(0.U(23.W))
  val configRegRamAddress = RegInit(0.U(19.W))
  val configRegRamMask = RegInit(0.U(17.W))
  val configRegImuAccelX = RegInit(0.U(16.W))
  val configRegImuAccelY = RegInit(0.U(16.W))
  val statRegStalls = RegInit(0.U(32.W))
  val statRegCycles = RegInit(0.U(32.W))

  val emuCartRtcAccess = Wire(new Mbc3RtcAccess)
  emuCartRtcAccess.writeEnable := false.B
  emuCartRtcAccess.writeState := DontCare
  emuCartRtcAccess.latchSelect := DontCare
  private def makeRtcAccess(latched: Boolean): RegisterMap.Entry = {
    RegisterMap.Entry(
      (new RtcState).getWidth,
      read = RegisterMap.ReadFn((read: Bool) => {
        when (read) { emuCartRtcAccess.latchSelect := latched.B }
        emuCartRtcAccess.readState.asUInt
      }),
      write = RegisterMap.WriteFn((write: Bool, data: UInt) =>
        when (write) {
          emuCartRtcAccess.latchSelect := latched.B
          emuCartRtcAccess.writeState := data.asTypeOf(new RtcState)
          emuCartRtcAccess.writeEnable := true.B
        }
      ),
    )
  }

  val registerInterface = Wire(new MemoryInterface(addressWidth = 16, dataWidth = 32))
  val biosInterface = Wire(new MemoryInterface(addressWidth = 11, dataWidth = 8)) // 2 KiB
  io.mcuInterface <> MemoryMap(
    addressWidth = 24,
    dataWidth = 32,
    entries = Seq(
      "b0000".U(4.W) -> registerInterface,
      "b0001".U(4.W) -> biosInterface,
    ))

  suppressEnumCastWarning {
    registerInterface <> RegisterMap(
      addressWidth = 16,
      dataWidth = 32,
      entries = Seq(
        0x0000 -> RegisterMap.Entry.rw(configRegSystem),
        0x0004 -> RegisterMap.Entry.rw(configRegEmuCart), // Suppressing mbcType enum cast
        0x0008 -> RegisterMap.Entry.rw(configRegRomAddress),
        0x000C -> RegisterMap.Entry.rw(configRegRomMask),
        0x0010 -> RegisterMap.Entry.rw(configRegRamAddress),
        0x0014 -> RegisterMap.Entry.rw(configRegRamMask),
        0x0018 -> makeRtcAccess(latched = false),
        0x001C -> makeRtcAccess(latched = true),
        0x0020 -> RegisterMap.Entry.rw(configRegImuAccelX),
        0x0024 -> RegisterMap.Entry.rw(configRegImuAccelY),

        0x1000 -> RegisterMap.Entry.rw(statRegStalls),
        0x1004 -> RegisterMap.Entry.rw(statRegCycles),
      )
    )
  }

  // Gameboy
  val gameboyConfig = Gameboy.Configuration(
    skipBootrom = false,
    optimizeForSimulation = false,
  )
  val gameboy = Module(new Gameboy(gameboyConfig))
  when (io.reset) {
    gameboy.reset := true.B
  }
  gameboy.io.isCgb := configRegSystem.isCgb

  // Gameboy clock control
  val doStall = WireDefault(false.B)
  gameboy.io.clockConfig.enable := false.B
  when (io.enable) {
    when (doStall) {
      statRegStalls := statRegStalls + 1.U
    }.otherwise {
      gameboy.io.clockConfig.enable := true.B
      statRegCycles := statRegCycles + 1.U
    }
  }
  gameboy.io.clockConfig.provide8Mhz := true.B

  gameboy.io.joypad.a := io.buttons.a
  gameboy.io.joypad.b := io.buttons.b
  gameboy.io.joypad.up := io.buttons.up
  gameboy.io.joypad.down := io.buttons.down
  gameboy.io.joypad.left := io.buttons.left
  gameboy.io.joypad.right := io.buttons.right
  gameboy.io.joypad.start := io.buttons.start
  gameboy.io.joypad.select := io.buttons.select

  // Vibration unused by default.
  io.vibrate := false.B

  // PMOD unused
  io.pmod.out := DontCare
  io.pmod.dir := 0.U(4.W)

  io.audioLeft := gameboy.io.apu.left << 6
  io.audioRight := gameboy.io.apu.right << 6

  // Link port
  io.link.soOut := gameboy.io.serial.out
  io.link.soDir := true.B
  gameboy.io.serial.in := RegNext(RegNext(io.link.siIn))
  io.link.siOut := DontCare
  io.link.siDir := false.B
  io.link.sdOut := DontCare
  io.link.sdDir := false.B
  gameboy.io.serial.clockIn := RegNext(RegNext(io.link.scIn))
  io.link.scOut := gameboy.io.serial.clockOut
  io.link.scDir := gameboy.io.serial.clockEnable

  // Framebuffer output
  val framebufferX = RegInit(0.U(8.W))
  val framebufferY = RegInit(0.U(8.W))
  io.framebufferX := framebufferX
  io.framebufferY := framebufferY
  io.framebufferWriteEnable := false.B
  io.framebufferData.a := DontCare
  io.framebufferData.r := DontCare
  io.framebufferData.g := DontCare
  io.framebufferData.b := DontCare
  io.vblank := gameboy.io.ppu.vblank

  val regDisplayOff = RegInit(true.B)
  when (regDisplayOff && framebufferY >= 144.U) {
    io.vblank := true.B
  }

  val prevHblank = RegInit(false.B)
  when (gameboy.io.clockConfig.enable) {
    prevHblank := gameboy.io.ppu.hblank

    when (regDisplayOff) {
      // Ensure that, when the display is turned off, it remains off until the next
      // vblank when the LCD is on. This ensures that the entire screen is blanked,
      // and matches Gameboy behavior.
      // Additionally, take care to ensure that spurious io.vblank transitions
      // don't happen (when moving between display on and off),
      // because this signals the triple buffering system that a frame is complete.
      io.vblank := false.B
      io.framebufferWriteEnable := true.B
      io.framebufferData.r := 0x1F.U(5.W)
      io.framebufferData.g := 0x1F.U(5.W)
      io.framebufferData.b := 0x1F.U(5.W)

      when (framebufferX < 159.U) {
        framebufferX := framebufferX + 1.U
      } .elsewhen (framebufferY < 143.U) {
        framebufferX := 0.U
        framebufferY := framebufferY + 1.U
      } .otherwise {
        io.framebufferWriteEnable := false.B
        io.vblank := true.B
      }

      // Only end blanking after the *next* vblank when the LCD is on
      when (gameboy.io.ppu.lcdEnable && gameboy.io.ppu.vblank) {
        regDisplayOff := false.B
      }
    } .otherwise {
      when (!gameboy.io.ppu.lcdEnable) {
        // Blank for at least a whole frame.
        regDisplayOff := true.B
        framebufferX := 0.U
        framebufferY := 0.U
      } .elsewhen (gameboy.io.ppu.vblank) {
        framebufferX := 0.U
        framebufferY := 0.U
      } .elsewhen (gameboy.io.ppu.hblank && !prevHblank) {
        framebufferX := 0.U
        framebufferY := framebufferY + 1.U
      } .elsewhen (gameboy.io.ppu.valid) {
        io.framebufferWriteEnable := true.B
        io.framebufferData.r := gameboy.io.ppu.pixel(4, 0)
        io.framebufferData.g := gameboy.io.ppu.pixel(9, 5)
        io.framebufferData.b := gameboy.io.ppu.pixel(14, 10)
        framebufferX := framebufferX + 1.U
      }
    }
  }

  // Emulated Cartridge
  val emuCart = Module(new EmuCartridge(8 * 1024 * 1024))
  when (io.reset) {
    emuCart.reset := true.B
  }
  emuCart.io.config := configRegEmuCart
  emuCart.io.tCycle := gameboy.io.tCycle
  emuCart.io.rtcAccess <> emuCartRtcAccess
  emuCart.io.imu.x := configRegImuAccelX
  emuCart.io.imu.y := configRegImuAccelY

  io.sdram.enable := false.B
  io.sdram.write := false.B
  io.sdram.address := DontCare
  io.sdram.dataWrite := DontCare
  io.sdram.writeStrobe := DontCare
  io.sram.enable := false.B
  io.sram.write := false.B
  io.sram.address := DontCare
  io.sram.dataWrite := DontCare
  io.sram.writeStrobe := DontCare

  val regEmuCartBusy = RegInit(false.B)
  val regEmuCartDataRead = Reg(UInt(8.W))
  val regEmuCartDataWrite = Reg(UInt(8.W))
  val regEmuCartAddress = Reg(UInt(23.W))
  val regEmuCartIsWrite = Reg(Bool())
  val regEmuCartSelectRom = Reg(Bool())
  val emuCartDataWrite = WireDefault(regEmuCartDataWrite)
  val emuCartIsWrite = WireDefault(regEmuCartIsWrite)
  val emuCartAddress = WireDefault(regEmuCartAddress)
  val emuCartSelectRom = WireDefault(regEmuCartSelectRom)
  val emuCartAccessStart = emuCart.io.dataAccess.enable && !emuCart.reset.asBool
  when (emuCartAccessStart) {
    regEmuCartBusy := true.B

    regEmuCartDataWrite := emuCart.io.dataAccess.dataWrite
    regEmuCartAddress := emuCart.io.dataAccess.address
    regEmuCartIsWrite := emuCart.io.dataAccess.write
    regEmuCartSelectRom := emuCart.io.dataAccess.selectRom

    emuCartDataWrite := emuCart.io.dataAccess.dataWrite
    emuCartAddress := emuCart.io.dataAccess.address
    emuCartIsWrite := emuCart.io.dataAccess.write
    emuCartSelectRom := emuCart.io.dataAccess.selectRom
  }
  emuCart.io.dataAccess.valid := false.B
  emuCart.io.dataAccess.dataRead := regEmuCartDataRead
  when (emuCartAccessStart || regEmuCartBusy) {
    when (emuCartSelectRom) {
      when (emuCartIsWrite) {
        // Don't handle ROM writes.
        emuCart.io.dataAccess.valid := true.B
      } .otherwise {
        io.sdram.enable := true.B
        io.sdram.write := false.B
        io.sdram.address := configRegRomAddress + (Cat(emuCartAddress(22, 2), "b00".U(2.W)) & configRegRomMask)
        emuCart.io.dataAccess.dataRead := io.sdram.dataRead
          .asTypeOf(Vec(4, UInt(8.W)))(
            emuCartAddress(1, 0)
          )
        emuCart.io.dataAccess.valid := io.sdram.done
      }
    } .otherwise {
      io.sram.enable := true.B
      io.sram.write := emuCartIsWrite
      io.sram.address := (configRegRamAddress + (Cat(emuCartAddress(16, 1), "b0".U(1.W)) & configRegRamMask)) >> 1
      io.sram.dataWrite := Fill(2, emuCartDataWrite)
      io.sram.writeStrobe := Mux(emuCartAddress(0), "b10".U(2.W), "b01".U(2.W))
      emuCart.io.dataAccess.valid := io.sram.done
      emuCart.io.dataAccess.dataRead := Mux(
        emuCartAddress(0),
        io.sram.dataRead(15, 8),
        io.sram.dataRead(7, 0)
      )
    }
  }
  when (regEmuCartBusy && emuCart.io.dataAccess.valid) {
    regEmuCartBusy := false.B
    regEmuCartDataRead := emuCart.io.dataAccess.dataRead
  }

  when (emuCart.io.config.enabled) {
    io.cartridgeEnabled := false.B

    // Connect emulated cartridge
    emuCart.io.cartridge <> gameboy.io.cartridge
    io.vibrate := emuCart.io.rumble
    doStall := emuCart.io.stall

    // Disconnect physical cartridge
    io.cartridge.bank0Out := DontCare
    io.cartridge.bank1Out := DontCare
    io.cartridge.bank2Out := DontCare
    io.cartridge.bank3Out := DontCare
    io.cartridge.pin30Out := DontCare
    io.cartridge.pin31Out := DontCare
    io.cartridge.bank0Dir := false.B
    io.cartridge.bank1Dir := false.B
    io.cartridge.bank2Dir := false.B
    io.cartridge.bank3Dir := false.B
    io.cartridge.pin30Dir := false.B
    io.cartridge.pin31Dir := false.B
  } .otherwise {
    // Cartridge I/O
    io.cartridgeEnabled := true.B

    // Bank 0: Data bus
    gameboy.io.cartridge.dataIn := io.cartridge.bank0In
    io.cartridge.bank0Out := gameboy.io.cartridge.dataOut
    io.cartridge.bank0Dir := gameboy.io.cartridge.dataDir

    // Bank 1: Address High
    io.cartridge.bank1Out := gameboy.io.cartridge.address(15, 8)
    io.cartridge.bank1Dir := true.B

    // Bank 2: Address Low
    io.cartridge.bank2Out := gameboy.io.cartridge.address(7, 0)
    io.cartridge.bank2Dir := true.B

    // Bank 3: Control signals (0: nCS, 1: nRD, 2: nWR, 3: PHI)
    io.cartridge.bank3Dir := true.B
    io.cartridge.bank3Out := Cat(
      gameboy.io.cartridge.phi,
      gameboy.io.cartridge.nWR,
      gameboy.io.cartridge.nRD,
      gameboy.io.cartridge.nCS,
    )

    // Pin 30: nRST
    // TODO: open-drain bidirectional
    io.cartridge.pin30Dir := true.B
    io.cartridge.pin30Out := gameboy.io.cartridge.nResetOut
    gameboy.io.cartridge.nResetIn := io.cartridge.pin30In

    // Pin 31: VIN
    io.cartridge.pin31Dir := false.B
    io.cartridge.pin31Out := DontCare

    // Disconnect emulated cartridge
    emuCart.io.cartridge := DontCare
    emuCart.io.cartridge.reqStart := false.B
  }

  // Boot ROM
  val bios = SRAM(2048, UInt(8.W), numReadPorts = 1, numWritePorts = 1, numReadwritePorts = 0)
  bios.writePorts(0).enable := biosInterface.enable && biosInterface.write
  bios.writePorts(0).address := biosInterface.address
  bios.writePorts(0).data := biosInterface.dataWrite
  biosInterface.dataRead := 0.U
  biosInterface.done := RegNext(bios.writePorts(0).enable || bios.readPorts(0).enable)
  bios.readPorts(0).enable := gameboy.io.bootRom.read
  bios.readPorts(0).address := gameboy.io.bootRom.address
  gameboy.io.bootRom.data := bios.readPorts(0).data
}