package platform.handheld

import chisel3._
import chisel3.util._
import gba.GBA
import gba.cart.emu.EmulatedCartridge
import lib.mem.cache.DirectReadCache
import lib.mem.{MemoryArbiter, MemoryInterface, MemoryMap, RegisterMap}

/**
 * Clocked by a 16777216 Hz clock.
 */
class HandheldGba extends Module with HandheldModule {
  val io = IO(new HandheldIo)
  def framebufferW = 240
  def framebufferH = 160
  def clockSystemHz = 16 * 1024 * 1024
  def clockSdramHz = clockSystemHz * 4

  val configRegEmuCart = RegInit(0.U.asTypeOf(new EmulatedCartridge.Config))
  val configRegRomSize = RegInit(0.U(25.W))
  val configRegGBPlayer = RegInit(0.U(1.W))
  val configRegImuGyroZ = RegInit(0.U(12.W))
  val configRegImuAccelX = RegInit(0.U(12.W))
  val configRegImuAccelY = RegInit(0.U(12.W))
  val statRegStalls = RegInit(0.U(32.W))
  val statRegCycles = RegInit(0.U(32.W))

  val rtcDataSelect = Wire(UInt(1.W))
  val rtcDataWrite = WireDefault(false.B)
  val rtcDataIn = Wire(UInt(32.W))
  val rtcDataOut = Wire(UInt(32.W))
  rtcDataSelect := DontCare
  rtcDataIn := DontCare
  private def makeRtcAccess(select: Int): RegisterMap.Entry = {
    RegisterMap.Entry(
      32,
      read = RegisterMap.ReadFn((read: Bool) => {
        when (read) { rtcDataSelect := select.U }
        rtcDataOut.asUInt
      }),
      write = RegisterMap.WriteFn((write: Bool, data: UInt) =>
        when (write) {
          rtcDataSelect := select.U
          rtcDataIn := data
          rtcDataWrite := true.B
        }
      ),
    )
  }

  val registerInterface = Wire(new MemoryInterface(addressWidth = 16, dataWidth = 32))
  val biosInterface = Wire(new MemoryInterface(addressWidth = 14, dataWidth = 32)) // 16 KiB
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
        0x0000 -> RegisterMap.Entry.rw(configRegEmuCart),
        // Rom size (minus one), max (2**25 - 1), 32MiB
        0x0004 -> RegisterMap.Entry.rw(configRegRomSize),
        0x0008 -> RegisterMap.Entry.rw(configRegGBPlayer),
        0x0100 -> RegisterMap.Entry.rw(configRegImuGyroZ),
        0x0104 -> RegisterMap.Entry.rw(configRegImuAccelX),
        0x0108 -> RegisterMap.Entry.rw(configRegImuAccelY),
        0x0200 -> makeRtcAccess(0),
        0x0204 -> makeRtcAccess(1),

        0x1000 -> RegisterMap.Entry.rw(statRegStalls),
        0x1004 -> RegisterMap.Entry.rw(statRegCycles),
      )
    )
  }

  io.vibrate := false.B

  // SDRAM interface and port
  val sdramCache = Module(new DirectReadCache(addressWidth = 23, dataWidth = 32, numEntries = 4096))
  io.sdram <> sdramCache.io.out
  io.sdram.address := sdramCache.io.out.address << 2
  val sdramPort = sdramCache.io.in
  sdramPort.enable := false.B
  sdramPort.address := DontCare
  sdramPort.write := false.B
  sdramPort.writeStrobe := DontCare
  sdramPort.dataWrite := DontCare

  // SRAM arbiter (shared between EWRAM and emucart)
  val sramArbiter = Module(new MemoryArbiter(addressWidth = 18, dataWidth = 16, n = 2))
  io.sram <> sramArbiter.io.target
  val sramEwram = sramArbiter.io.initiator(0)
  val sramEmuCart = sramArbiter.io.initiator(1)

  // Gameboy
  val gba = Module(new GBA)
  when (io.reset) {
    gba.reset := true.B
  }
  val doStall = WireDefault(false.B)
  gba.io.enable := false.B
  when (io.enable) {
    when (doStall) {
      statRegStalls := statRegStalls + 1.U
    }.otherwise {
      gba.io.enable := true.B
      statRegCycles := statRegCycles + 1.U
    }
  }

  gba.io.configGBPlayer := configRegGBPlayer.asBool
  when (gba.io.configGBPlayer && gba.io.gbpRumble) {
    io.vibrate := true.B
  }

  // Emulated cartridge
  val emuCart = Module(new EmulatedCartridge)
  when (io.reset) {
    emuCart.reset := true.B
    sdramCache.reset := true.B
  }
  emuCart.io.config := configRegEmuCart
  emuCart.io.romSize := configRegRomSize
  emuCart.io.imuGyroZ := configRegImuGyroZ
  emuCart.io.imuAccelX := configRegImuAccelX
  emuCart.io.imuAccelY := configRegImuAccelY

  emuCart.io.rtcDataWrite := rtcDataWrite
  emuCart.io.rtcDataIn := rtcDataIn
  emuCart.io.rtcDataSelect := rtcDataSelect
  rtcDataOut := emuCart.io.rtcDataOut

  // Convert 16-bit addresses to 32-bit byte addresses
  // Also dealing with enable = true when done = true
  // Note however that EmulatedCartridge will never do that, because
  // reqEnd goes high, then the next cycle reqStart can go high again
  val emuCartBusy = RegInit(false.B)
  val emuCartAddr = Reg(UInt(24.W))
  val emuCartData = Reg(UInt(16.W))
  emuCart.io.rom.done := sdramPort.done
  emuCart.io.rom.dataRead := emuCartData
  when (emuCartBusy) {
    sdramPort.enable := true.B
    sdramPort.address := emuCartAddr >> 1
    when (sdramPort.done) {
      emuCartBusy := false.B
      val data = sdramPort.dataRead.asTypeOf(Vec(2, UInt(16.W)))(emuCartAddr(0))
      emuCartData := data
      emuCart.io.rom.dataRead := data
    }
  } .elsewhen (emuCart.io.rom.enable) {
    sdramPort.enable := true.B
    sdramPort.address := emuCart.io.rom.address >> 1
    emuCartAddr := emuCart.io.rom.address
    emuCartBusy := true.B
  }
  // Emulated cartridge SRAM: convert 8-bit accesses to 16-bit. Starts at 0 bytes into SRAM (takes 128KiB / 512 KiB).
  val regEmuCartSramByte = RegEnable(emuCart.io.backup.address(0), emuCart.io.backup.enable)
  sramEmuCart.enable := emuCart.io.backup.enable
  sramEmuCart.address := emuCart.io.backup.address >> 1
  sramEmuCart.write := emuCart.io.backup.write
  sramEmuCart.dataWrite := Fill(2, emuCart.io.backup.dataWrite)
  sramEmuCart.writeStrobe := Mux(emuCart.io.backup.address(0), "b10".U(2.W), "b01".U(2.W))
  emuCart.io.backup.done := sramEmuCart.done
  emuCart.io.backup.dataRead := sramEmuCart.dataRead.asTypeOf(Vec(2, UInt(8.W)))(regEmuCartSramByte)

  // Cartridge
  when (configRegEmuCart.enabled) {
    // Connect emulated cartridge
    gba.io.cartridge <> emuCart.io.interface
    doStall := emuCart.io.stall || gba.io.ewramStall

    when (emuCart.io.vibrate) {
      io.vibrate := true.B
    }

    // Disconnect physical cartridge
    io.cartridgeEnabled := false.B
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
    doStall := gba.io.ewramStall

    io.cartridgeEnabled := true.B
    io.cartridge.bank0Dir := gba.io.cartridge.AHiDir
    io.cartridge.bank0Out := gba.io.cartridge.AHiOut
    gba.io.cartridge.AHiIn := io.cartridge.bank0In
    io.cartridge.bank1Dir := gba.io.cartridge.ADLoDir
    io.cartridge.bank1Out := gba.io.cartridge.ADLoOut(15, 8)
    io.cartridge.bank2Dir := gba.io.cartridge.ADLoDir
    io.cartridge.bank2Out := gba.io.cartridge.ADLoOut(7, 0)
    gba.io.cartridge.ADLoIn := Cat(io.cartridge.bank1In, io.cartridge.bank2In)

    io.cartridge.bank3Dir := true.B
    io.cartridge.bank3Out := Cat(
      gba.io.cartridge.phi,
      gba.io.cartridge.nWR,
      gba.io.cartridge.nRD,
      gba.io.cartridge.nCS,
    )
    io.cartridge.pin30Dir := true.B
    io.cartridge.pin30Out := gba.io.cartridge.nCS2
    io.cartridge.pin31Dir := false.B
    io.cartridge.pin31Out := DontCare
    gba.io.cartridge.IRQ := io.cartridge.pin31In

    // Disconnected emulated cartridge
    emuCart.io.interface.phi := false.B
    emuCart.io.interface.nWR := true.B
    emuCart.io.interface.nRD := true.B
    emuCart.io.interface.nCS := true.B
    emuCart.io.interface.ADLoOut := DontCare
    emuCart.io.interface.ADLoDir := DontCare
    emuCart.io.interface.AHiOut := DontCare
    emuCart.io.interface.AHiDir := DontCare
    emuCart.io.interface.nCS2 := true.B
    emuCart.io.interface.reqStart := false.B
    emuCart.io.interface.reqRom := DontCare
    emuCart.io.interface.reqWrite := DontCare
    emuCart.io.interface.reqAddress := DontCare
    emuCart.io.interface.reqEnd := false.B
  }

  // Video output
  val framebufferX = RegInit(0.U(8.W))
  val framebufferY = RegInit(0.U(8.W))
  io.framebufferX := framebufferX
  io.framebufferY := framebufferY
  io.framebufferWriteEnable := false.B
  io.framebufferData.a := DontCare
  io.framebufferData.r := DontCare
  io.framebufferData.g := DontCare
  io.framebufferData.b := DontCare
  io.vblank := gba.io.ppu.vblank

  val prevHblank = RegInit(false.B)
  when (gba.io.enable) {
    prevHblank := gba.io.ppu.hblank
    when (gba.io.ppu.vblank) {
      framebufferX := 0.U
      framebufferY := 0.U
    } .elsewhen (gba.io.ppu.hblank && !prevHblank) {
      framebufferX := 0.U
      framebufferY := framebufferY + 1.U
    } .elsewhen (gba.io.ppu.valid) {
      io.framebufferWriteEnable := true.B
      io.framebufferData.r := gba.io.ppu.pixel(4, 0)
      io.framebufferData.g := gba.io.ppu.pixel(9, 5)
      io.framebufferData.b := gba.io.ppu.pixel(14, 10)
      framebufferX := framebufferX + 1.U
    }
  }

  // Audio output
  io.audioLeft := gba.io.apu.left << 6
  io.audioRight := gba.io.apu.right << 6

  // Keypad
  gba.io.keypad.a := io.buttons.a
  gba.io.keypad.b := io.buttons.b
  gba.io.keypad.l := io.buttons.l
  gba.io.keypad.r := io.buttons.r
  gba.io.keypad.up := io.buttons.up
  gba.io.keypad.down := io.buttons.down
  gba.io.keypad.left := io.buttons.left
  gba.io.keypad.right := io.buttons.right
  gba.io.keypad.start := io.buttons.start
  gba.io.keypad.select := io.buttons.select

  // BIOS
  val bios = SRAM(16 * 1024 / 4, UInt(32.W), numReadPorts = 1, numWritePorts = 1, numReadwritePorts = 0)
  bios.writePorts(0).enable := biosInterface.enable && biosInterface.write
  bios.writePorts(0).address := biosInterface.address >> 2
  bios.writePorts(0).data := biosInterface.dataWrite
  biosInterface.dataRead := 0.U
  biosInterface.done := RegNext(bios.writePorts(0).enable || bios.readPorts(0).enable)
  bios.readPorts(0).enable := gba.io.biosRom.read
  bios.readPorts(0).address := gba.io.biosRom.address
  gba.io.biosRom.data := bios.readPorts(0).data

  // EWRAM. Starts at 256KB into the external SRAM.
  sramEwram <> gba.io.ewram
  sramEwram.address := Cat(1.U(1.W), gba.io.ewram.address)

  io.pmod.out := gba.io.link.in.asUInt
  io.pmod.dir := "b1111".U(4.W)

  // Link port
  io.link.scOut := RegNext(gba.io.link.out.sc)
  io.link.sdOut := RegNext(gba.io.link.out.sd)
  io.link.siOut := RegNext(gba.io.link.out.si)
  io.link.soOut := RegNext(gba.io.link.out.so)
  io.link.scDir := RegNext(gba.io.link.dir.sc)
  io.link.sdDir := RegNext(gba.io.link.dir.sd)
  io.link.siDir := RegNext(gba.io.link.dir.si)
  io.link.soDir := RegNext(gba.io.link.dir.so)
  gba.io.link.in.sc := RegNext(RegNext(io.link.scIn))
  gba.io.link.in.sd := RegNext(RegNext(io.link.sdIn))
  gba.io.link.in.si := RegNext(RegNext(io.link.siIn))
  gba.io.link.in.so := RegNext(RegNext(io.link.soIn))
}