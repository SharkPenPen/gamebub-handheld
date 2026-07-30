package platform.sim

import chisel3._
import _root_.circt.stage.ChiselStage
import chisel3.util.SRAM
import gba._
import gba.apu.ApuOutput
import gba.cart.emu.{EmulatedCartridge, VerificationEmulatedCartridge}
import gba.ppu.PpuOutput
import lib.log.Log
import lib.mem.MemoryInterface

object SimGba extends App {
  Log.setDefaultLevel(Log.Level.Warning)
  sys.env.get("LOG_LEVELS") match {
    case Some(value) => Log.setLevelsFromString(value)
    case None =>
  }

  ChiselStage.emitSystemVerilogFile(
    new SimGba,
    args,
    firtoolOpts = Array(
      "--preserve-aggregate=1d-vec",
    )
  )
}

class SimGba extends Module {
  val io = IO(new Bundle {
    val enable = Input(Bool())
    val ppu = Output(new PpuOutput)
    val keypad = Input(new Keypad.State)
    val apu = Output(new ApuOutput)

    val configGBPlayer = Input(Bool())

    val emuCartConfig = Input(new EmulatedCartridge.Config)
    val emuCartRom = Flipped(new MemoryInterface(addressWidth = 24, dataWidth = 16))
    val emuCartRomSize = Input(UInt(25.W))
    val emuCartBackup = Flipped(new MemoryInterface(addressWidth = 17, dataWidth = 8))
    val emuCartStall = Output(Bool())
  })

  val gba = Module(new GBA())
  gba.io.enable <> io.enable
  gba.io.configGBPlayer := io.configGBPlayer
  gba.io.ppu <> io.ppu
  gba.io.keypad <> io.keypad
  gba.io.apu <> io.apu

  // BIOS, to be filled in by verilator simulator
  val biosRom = {
    val rom = SyncReadMem(16 * 1024 / 4, UInt(32.W))
    // dontTouch: hack to ensure Chisel doesn't optimize the mem out
    val temp = dontTouch(WireDefault(false.B))
    when (temp) {
      rom.write(0.U, 0.U)
    }
    rom
  }
  gba.io.biosRom.data := biosRom.read(gba.io.biosRom.address, gba.io.biosRom.read)

  // EWRAM
  val ewram = SRAM.masked(128 * 1024, Vec(2, UInt(8.W)), numReadPorts = 0, numWritePorts = 0, numReadwritePorts = 1)
  val ewramPort = ewram.readwritePorts(0)
  ewramPort.address := gba.io.ewram.address
  ewramPort.enable := gba.io.ewram.enable
  ewramPort.isWrite := gba.io.ewram.write
  ewramPort.mask.get := gba.io.ewram.writeStrobe.asBools
  ewramPort.writeData := gba.io.ewram.dataWrite.asTypeOf(ewramPort.writeData)
  gba.io.ewram.dataRead := ewramPort.readData.asUInt
  gba.io.ewram.done := true.B

  // Emulated cartridge
  val emuCart = Module(new EmulatedCartridge)
  emuCart.io.config := io.emuCartConfig
  emuCart.io.romSize := io.emuCartRomSize
  emuCart.io.imuAccelX := 0x3A0.U
  emuCart.io.imuAccelY := 0x3A0.U
  emuCart.io.imuGyroZ := DontCare  // No gyro support in simulator
  emuCart.io.rtcDataWrite := false.B
  emuCart.io.rtcDataIn := DontCare
  emuCart.io.rtcDataSelect := DontCare
  io.emuCartStall := emuCart.io.stall
  gba.io.cartridge <> emuCart.io.interface
  io.emuCartRom <> emuCart.io.rom
  io.emuCartBackup <> emuCart.io.backup

  // Link
  gba.io.link.in.si := false.B
  gba.io.link.in.so := true.B
  gba.io.link.in.sc := true.B
  gba.io.link.in.sd := true.B
}