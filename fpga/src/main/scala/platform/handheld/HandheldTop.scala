package platform.handheld

import chisel3._
import chisel3.util._
import _root_.circt.stage.ChiselStage
import lib.mem.{MemoryArbiter, MemoryCdc, MemoryInterface, MemoryMap, RegisterMap}
import lib.video.{ColorARGB, HdmiTransmitter}
import xilinx.{XpmCdcHandshake, XpmCdcSingle, XpmCdcSyncRst}

object HandheldTop extends App {
  // Parse arguments.
  if (args.length < 1) {
    throw new IllegalArgumentException("missing arg 0: inner class")
  }
  val argInnerClassName = args.head
  val argRest = args.tail

  // Generate verilog.
  val moduleFactory = () =>
    Class
      .forName(argInnerClassName)
      .getDeclaredConstructor()
      .newInstance()
      .asInstanceOf[Module with HandheldModule]

  ChiselStage.emitSystemVerilogFile(
    new HandheldTop(moduleFactory()),
    argRest,
    firtoolOpts = Array(
      "--preserve-aggregate=1d-vec",
    )
  )
}

/** IO bundle used for a handheld submodule. */
class HandheldIo extends Bundle {
  val enable = Input(Bool())
  val reset = Input(Bool())

  val buttons = Input(new HandheldButtons)

  // Video output
  val framebufferX = Output(UInt(8.W))
  val framebufferY = Output(UInt(8.W))
  val framebufferData = Output(ColorARGB.rgb555())
  val framebufferWriteEnable = Output(Bool())
  val vblank = Output(Bool())

  // Audio output
  val audioLeft = Output(SInt(16.W))
  val audioRight = Output(SInt(16.W))

  // Vibration
  val vibrate = Output(Bool())

  // Cartridge
  val cartridgeEnabled = Output(Bool())
  val cartridge = new HandheldCartridge

  val link = new HandheldLink
  val pmod = new HandheldPmod

  val mcuInterface = new MemoryInterface(addressWidth = 30, dataWidth = 32)

  // Memory interfaces
  val sram = Flipped(new MemoryInterface(addressWidth = 18, dataWidth = 16))
  val sdram = Flipped(new MemoryInterface(addressWidth = 25, dataWidth = 32))
}

trait HandheldModule {
  def io: HandheldIo

  def framebufferW: Int
  def framebufferH: Int
  def clockSystemHz: Int
  def clockSdramHz: Int
}

/** Buttons on the handheld. All are active-high. */
class HandheldButtons extends Bundle {
  val a = Bool()
  val b = Bool()
  val x = Bool()
  val y = Bool()
  val up = Bool()
  val down = Bool()
  val left = Bool()
  val right = Bool()
  val l = Bool()
  val r = Bool()
  val start = Bool()
  val select = Bool()
}

/**
 * Cartridge I/O for the handheld.
 *
 * Bank 0: A16 to A23
 * Bank 1: AD8 to AD15
 * Bank 2: AD0 to AD7
 * Bank 3:
 *  0: nCS1
 *  1: nRD
 *  2: nWR
 *  3: PHI
 * Pin 30: nRST (GB) / nCS2 (GBA)
 * Pin 31: VIN (GB) / nIRQ (GBA)
 *
 * Directions are all 1 for output, 0 for input.
 */
class HandheldCartridge extends Bundle {
  val bank0In = Input(UInt(8.W))
  val bank1In = Input(UInt(8.W))
  val bank2In = Input(UInt(8.W))
  val bank3In = Input(UInt(4.W))
  val pin30In = Input(Bool())
  val pin31In = Input(Bool())

  val bank0Out = Output(UInt(8.W))
  val bank1Out = Output(UInt(8.W))
  val bank2Out = Output(UInt(8.W))
  val bank3Out = Output(UInt(4.W))
  val pin30Out = Output(Bool())
  val pin31Out = Output(Bool())

  val bank0Dir = Output(Bool())
  val bank1Dir = Output(Bool())
  val bank2Dir = Output(Bool())
  val bank3Dir = Output(Bool())
  val pin30Dir = Output(Bool())
  val pin31Dir = Output(Bool())
}

class HandheldPmod extends Bundle {
  val in = Input(UInt(4.W))
  val out = Output(UInt(4.W))
  val dir = Output(UInt(4.W))
}

class HandheldLink extends Bundle {
  val soIn = Input(Bool())
  val siIn = Input(Bool())
  val sdIn = Input(Bool())
  val scIn = Input(Bool())
  val soOut = Output(Bool())
  val siOut = Output(Bool())
  val sdOut = Output(Bool())
  val scOut = Output(Bool())
  val soDir = Output(Bool())
  val siDir = Output(Bool())
  val sdDir = Output(Bool())
  val scDir = Output(Bool())
}

class HandheldInterrupts extends Bundle {
  val spiResponseFifoUnderflow = Bool()
  val spiRequestFifoOverflow = Bool()
  val moduleVblank = Bool()
}

/**
 * Top-level Chisel module for the handheld.
 *
 * The outer clock is passed down to the inner module,
 * e.g. 8.3886 MHz for Gameboy.
 */
class HandheldTop[T <: Module with HandheldModule](genT: => T) extends Module {
  val module = Module(genT)
  val sdramConfig = SdramController.Config(
    clockFrequency = module.clockSdramHz,
    burstLength = 2,
    timeRsc = (2 * 1_000_000_000) / module.clockSdramHz, /* 2 clocks */
    timeWr = (2 * 1_000_000_000) / module.clockSdramHz, /* 2 clocks */
  )
  val io = IO(new Bundle {
    /** Audio/video clock: 12.288 MHz when HDMI disabled, 27.027 MHz when HDMI enabled */
    val clock_av = Input(Clock())

    /** SPI clocking */
    val clockSpi = Input(Clock())
    val clockSpiLocked = Input(Bool())
    val clockSpiPowerDown = Output(Bool())

    /** MCU interrupt: true to pull it low (active) */
    val mcuIrq = Output(Bool())
    val mcuSpiChipSelect = Input(Bool())
    val mcuSpiClock = Input(Bool())
    val mcuSpiDataIn = Input(UInt(4.W))
    val mcuSpiDataOut = Output(UInt(4.W))
    val mcuSpiDataDir = Output(UInt(4.W))

    val lcd = Output(new DpiSignals)
    val lcdData = Output(UInt(18.W))
    val dac = Output(new I2sSignals)

    /** HDMI */
    val hdmiEnable = Output(Bool())
    val hdmiAudioClock = Output(Clock())
    val hdmiAudio = Output(Vec(2, UInt(16.W)))
    val hdmiRgb = Output(UInt(24.W))
    val hdmiCx = Input(UInt(10.W))
    val hdmiCy = Input(UInt(10.W))

    /** Raw button input, not registered or inverted. */
    val buttons = Input(new HandheldButtons)

    // Cartridge I/O
    /** Cartridge switch: 1 when DMG/CGB cartridge inserted */
    val cartridgeSwitch = Input(Bool())
    val cartridge3V3Enable = Output(Bool())
    val cartridge5V0Enable = Output(Bool())
    /** Cartridge shifter output enable: active-low */
    val cartridgeOutputEnableN = Output(Bool())
    val cartridge = new HandheldCartridge

    val vibrate = Output(Bool())
    val pmod = new HandheldPmod
    val link = new HandheldLink

    // SRAM
    val sram = new AsyncSramController.Signals(addressWidth = 18, dataWidth = 16)

    // SDRAM
    val sdramClock = Input(Clock())
    val sdram = new SdramController.Signals(sdramConfig)
  })

  val statNumDuplicatedFrames = Wire(UInt(24.W))
  val statNumSkippedFrames = Wire(UInt(24.W))

  //////////////////////////////////
  // MCU Communication
  //////////////////////////////////
  // D0: PICO, D1: POCI
  val spi = Module(new SpiReceiverFifo())
  spi.io.clockSpi := io.clockSpi
  spi.io.clockSpiLocked := io.clockSpiLocked
  io.clockSpiPowerDown := spi.io.clockSpiPowerDown
  io.mcuSpiDataDir := Mux(io.mcuSpiChipSelect, 0.U, spi.io.signals.serialDir)
  io.mcuSpiDataOut := spi.io.signals.serialOut
  spi.io.signals.serialClock := io.mcuSpiClock
  spi.io.signals.serialIn := io.mcuSpiDataIn
  spi.io.signals.chipSelect := io.mcuSpiChipSelect

  val controlRegister = RegInit(0.U.asTypeOf(new Bundle() {
    /** True to enable vibration (if the module uses it) */
    val vibrate = Bool()
    /** Whether the module is currently in vblank. (TODO make read-only) */
    val moduleVblank = Bool()
    /** Active-low reset for the inner module. */
    val moduleReset = Bool()
    /** Active-high enable for the inner module. */
    val moduleEnable = Bool()
  }))
  val displayRegister = RegInit(0.U.asTypeOf(new Bundle() {
    val enableHdmi = Bool()
  }))
  val buttonRegister = RegInit(0.U.asTypeOf(new HandheldButtons))
  val interruptEnable = RegInit(0.U.asTypeOf(new HandheldInterrupts))
  val interruptFlags = RegInit(0.U.asTypeOf(new HandheldInterrupts))
  val statusRegister = Cat(
    // 0: cartridge switch state
    RegNext(RegNext(io.cartridgeSwitch)),
  )

  val overlayXControlRegister = RegInit(0.U.asTypeOf(new Bundle() {
    val start = UInt(8.W)
    val end = UInt(8.W)
    val scroll = UInt(8.W)
  }))
  val overlayYControlRegister = RegInit(0.U.asTypeOf(new Bundle() {
    val start = UInt(8.W)
    val end = UInt(8.W)
    val scroll = UInt(8.W)
  }))

  val registerMap = RegisterMap(
    addressWidth = 16,
    dataWidth = 32,
    entries = Seq(
      0x0 -> RegisterMap.Entry.rw(controlRegister),
      0x4 -> RegisterMap.Entry.rw(buttonRegister),
      0x8 -> RegisterMap.Entry.rw(displayRegister),
      0xC -> RegisterMap.Entry.rw(interruptEnable),
      0x10 -> RegisterMap.Entry(
        interruptFlags.getWidth,
        read = RegisterMap.ReadFn((_: Bool) => interruptFlags.asUInt),
        write = RegisterMap.WriteFn((write: Bool, data: UInt) =>
          when (write) {
            // Write set bits to ack interrupts.
            interruptFlags := (interruptFlags.asUInt & (~data).asUInt).asTypeOf(interruptFlags)
          }
        ),
      ),
      0x14 -> RegisterMap.Entry.r(statusRegister),
      // Overlay control
      0x100 -> RegisterMap.Entry.rw(overlayXControlRegister),
      0x104 -> RegisterMap.Entry.rw(overlayYControlRegister),
      // Framebuffer dimensions
      0x200 -> RegisterMap.Entry.r(
        Cat(module.framebufferW.U(16.W), module.framebufferH.U(16.W))),
      // Stats
      0x300 -> RegisterMap.Entry.r(statNumDuplicatedFrames),
      0x304 -> RegisterMap.Entry.r(statNumSkippedFrames),
    )
  )

  val sramSpiInterface = Wire(new MemoryInterface(addressWidth = 19, dataWidth = 16))
  val sdramSpiInterface = Wire(new MemoryInterface(addressWidth = 25, dataWidth = 32))
  val moduleMcuInterface = Wire(new MemoryInterface(addressWidth = 30, dataWidth = 32))
  val overlayInterface = Wire(new MemoryInterface(addressWidth = 18, dataWidth = 16))
  val framebufferInterface = Wire(new MemoryInterface(addressWidth = 18, dataWidth = 16))
  spi.io.mem <> MemoryMap(
    addressWidth = 32,
    dataWidth = 32,
    entries = Seq(
      "b0000".U(4.W) -> registerMap,
      "b0001".U(4.W) -> sramSpiInterface,
      "b0010".U(4.W) -> sdramSpiInterface,
      "b001110".U(6.W) -> overlayInterface,
      "b001111".U(6.W) -> framebufferInterface,
      "b11".U(2.W) -> moduleMcuInterface,
    ))

  controlRegister.moduleVblank := module.io.vblank
  when (spi.io.debugRequestOverflow) {
    interruptFlags.spiRequestFifoOverflow := true.B

  }
  when (spi.io.debugResponseUnderflow) {
    interruptFlags.spiResponseFifoUnderflow := true.B
  }

  //////////////////////////////////
  // Interrupts
  //////////////////////////////////
  io.mcuIrq := (interruptFlags.asUInt & interruptEnable.asUInt).orR
  when (module.io.vblank && !RegNext(module.io.vblank)) {
    interruptFlags.moduleVblank := true.B
  }

  //////////////////////////////////
  // Memory
  //////////////////////////////////
  val sram = Module(new AsyncSramController(addressWidth = 18, dataWidth = 16))
  val sramArbiter = Module(new MemoryArbiter(addressWidth = 18, dataWidth = 16, n = 2))
  io.sram <> sram.io.signals
  sram.io.mem <> sramArbiter.io.target
  sramArbiter.io.initiator(0) <> sramSpiInterface
  sramArbiter.io.initiator(0).address := sramSpiInterface.address >> 1  // SPI is byte addressed

  // SDRAM
  val sdramArbiter = Module(new MemoryArbiter(addressWidth = 25, dataWidth = 32, n = 2))
  sdramArbiter.io.initiator(0) <> sdramSpiInterface

  val sdram = withClock(io.sdramClock) {
    Module(new SdramController(sdramConfig))
  }
  io.sdram <> sdram.io.signals

  withClock(io.sdramClock) {
    val cdc = Module(new MemoryCdc(addressWidth = 25, dataWidth = 32))
    cdc.io.slowClock := clock
    cdc.io.initiator <> sdramArbiter.io.target
    cdc.io.target <> sdram.io.mem
  }

  //////////////////////////////////
  // Video
  //////////////////////////////////
  val videoWidth = module.framebufferW
  val videoHeight = module.framebufferH

  io.hdmiEnable := displayRegister.enableHdmi

  // Triple buffering
  val framebuffers = (0 until 3).map(_ =>
    SRAM(
      videoWidth * videoHeight, UInt(ColorARGB.rgb555().getWidth.W),
      readPortClocks = Seq(io.clock_av), writePortClocks = Seq(), readwritePortClocks = Seq(clock)
    )
  )
  val framebufferControl = Module(new TripleBufferControl)
  statNumDuplicatedFrames := framebufferControl.io.statNumDuplicated
  statNumSkippedFrames := framebufferControl.io.statNumSkipped

  val overlayWidth = 240
  val overlayHeight = 160
  val overlayFramebuffer = SRAM(
    overlayWidth * overlayHeight, UInt(ColorARGB.argb1555().getWidth.W),
    readPortClocks = Seq(io.clock_av), writePortClocks = Seq(clock), readwritePortClocks = Seq(),
  )
  val reset_av = withClock(io.clock_av) { XpmCdcSyncRst(reset) }
  withClockAndReset (clock = io.clock_av, reset = reset_av) {
    val videoX = Wire(UInt(10.W))
    val videoY = Wire(UInt(10.W))
    val framebufferActive = Wire(Bool())
    val framebufferReadAddress = Wire(UInt(16.W))
    val overlayReadAddress = Wire(UInt(16.W))

    val audioData = XpmCdcHandshake.continuous(clock, Cat(module.io.audioLeft.asUInt, module.io.audioRight.asUInt))
    val audioDataLeft = audioData(31, 16)
    val audioDataRight = audioData(15, 0)

    // Buffering the read allows this to be a block ram instead of distributed ram
    // and an additional output buffer allows Vivado to improve timing.
    //
    // Read from the correct framebuffer.
    withClock (clock) {
      framebufferControl.io.readActive := XpmCdcSingle(io.clock_av, framebufferActive)
    }
    val framebufferIndex = XpmCdcHandshake.continuous(clock, framebufferControl.io.readIndex)
    for (i <- 0 until 3) {
      framebuffers(i).readPorts(0).enable := framebufferIndex === i.U
      framebuffers(i).readPorts(0).address := framebufferReadAddress
    }
    val framebufferRead = MuxLookup(framebufferIndex, 0.U)(
      (0 until 3).map(i => i.U -> RegNext(RegNext(framebuffers(i).readPorts(0).data)))
    ).asTypeOf(ColorARGB.rgb555())

    // Similar for overlay framebuffer.
    val overlayXControl = XpmCdcHandshake.continuous(clock, overlayXControlRegister)
    val overlayYControl = XpmCdcHandshake.continuous(clock, overlayYControlRegister)

    overlayFramebuffer.readPorts(0).enable := true.B
    overlayFramebuffer.readPorts(0).address := overlayReadAddress
    val overlayRead = RegNext(RegNext(overlayFramebuffer.readPorts(0).data)).asTypeOf(ColorARGB.argb1555())

    val framebufferInBounds = Wire(Bool())
    val overlayInBounds = Wire(Bool())
    val videoOutput = ColorARGB.rgb555().makeBlack()
    when (framebufferInBounds) {
      videoOutput := framebufferRead
    }
    when (overlayRead.a.asBool && overlayInBounds) {
      videoOutput := overlayRead
    }

    /**
     * DPI video signal output
     * dotclk = 12.288MHz, fps = 60
     * H = 320, total inactive = 88
     * V = 480, total inactive = 22
     */
    val dpiDriver = Module(new DpiDriver(
      hActive = 320,
      hSync = 30, // min = 3
      hBackPorch = 29, // min = 3
      hFrontPorch = 29, // min = 3
      vActive = 480,
      vSync = 8, // min = 1
      vBackPorch = 7, // min = 2
      vFrontPorch = 7, // min = 2
    ))
    io.lcd := dpiDriver.io.signals
    // Pad to 18-bit RGB.
    io.lcdData := Cat(
      Cat(videoOutput.r, 0.U(1.W)),
      Cat(videoOutput.g, 0.U(1.W)),
      Cat(videoOutput.b, 0.U(1.W)),
    )

    val i2sTransmitter =
      Module(new I2sTransmitter(
        bitWidth = 16,
        mclkFactor = 256,
        channels = 2,
      ))
    io.dac := i2sTransmitter.io.signals
    i2sTransmitter.io.dataLeft := audioDataLeft
    i2sTransmitter.io.dataRight := audioDataRight

    /**
     * HDMI audio and video signal output
     * Video ID Code 2: 720x480 @ 60Hz
     */
    val hdmiFrameWidth = 858
    val hdmiFrameHeight = 525
    io.hdmiAudio := VecInit(audioDataLeft, audioDataRight)
    io.hdmiAudioClock := DontCare
    // Pad to 24-bit RGB.
    io.hdmiRgb := Cat(
      videoOutput.r << 3,
      videoOutput.g << 3,
      videoOutput.b << 3,
    )

    val hdmiEnable = XpmCdcSingle(clock, displayRegister.enableHdmi)
    when (hdmiEnable) {
      dpiDriver.reset := true.B
      i2sTransmitter.reset := true.B
      val screenWidth = 720
      val screenHeight = 480

      // Correct HDMI video X and Y
      videoX := io.hdmiCx
      videoY := io.hdmiCy
      when (io.hdmiCx >= screenWidth.U) {
        // Make it so that adding wraps around to 0.
        // (frameWidth - 1) should be (2**width - 1)
        videoX := io.hdmiCx + ((1 << io.hdmiCx.getWidth) - hdmiFrameWidth).U
        videoY := io.hdmiCy + 1.U
        when (io.hdmiCy === (hdmiFrameHeight - 1).U) {
          videoY := 0.U
        }
      }
      framebufferActive := io.hdmiCy < screenHeight.U || io.hdmiCy === (hdmiFrameHeight - 1).U

      // Scale and center framebuffer within output video.
      val videoScale = 3
      val videoOffsetX = (screenWidth - (videoWidth * videoScale)) / 2
      val videoOffsetY = (screenHeight - (videoHeight * videoScale)) / 2
      val framebufferReadDelay = 3
      framebufferReadAddress :=
        (((videoY - videoOffsetY.U) / videoScale.U) * videoWidth.U) +
          ((videoX - videoOffsetX.U + framebufferReadDelay.U) / videoScale.U)
      framebufferInBounds := videoX >= videoOffsetX.U &&
        videoX < (videoOffsetX + (videoWidth * videoScale)).U &&
        videoY >= videoOffsetY.U &&
        videoY < (videoOffsetY + (videoHeight * videoScale)).U

      // Scale overlay
      val overlayScale = 3
      val overlayReadDelay = 3
      overlayReadAddress :=
        ((((videoY) / overlayScale.U) + overlayYControl.scroll)(7, 0) * overlayWidth.U) +
          (((videoX + overlayReadDelay.U) / overlayScale.U) + overlayXControl.scroll)(7, 0)
      overlayInBounds := videoX >= (overlayXControl.start * overlayScale.U) &&
        videoX < (overlayXControl.end * overlayScale.U) &&
        videoY >= (overlayYControl.start * overlayScale.U) &&
        videoY < (overlayYControl.end * overlayScale.U)

      // HDMI Audio
      val audioClock = RegInit(false.B)
      val audioCounter = Counter(27027000 / (48000 * 2))
      when (audioCounter.inc()) {
        audioClock := !audioClock
      }
      io.hdmiAudioClock := audioClock.asClock
    } .otherwise {
      val screenWidth = 480
      val screenHeight = 320

      val dpiX = dpiDriver.io.pixelY
      val dpiY = dpiDriver.io.pixelX
      videoX := dpiX
      videoY := dpiY
      framebufferActive := dpiX < screenWidth.U || dpiX === ((1 << dpiX.getWidth) - 1).U

      // Scale and center framebuffer without output video.
      val videoScale = 2
      val videoOffsetX = (screenWidth - (videoWidth * videoScale)) / 2
      val videoOffsetY = (screenHeight - (videoHeight * videoScale)) / 2
      val framebufferReadDelay = 3 // 3 cycles to read from the framebuffer
      framebufferReadAddress :=
        (((dpiY - videoOffsetY.U + framebufferReadDelay.U) / videoScale.U) * videoWidth.U) +
          ((dpiX - videoOffsetX.U) / videoScale.U)
      framebufferInBounds := dpiX >= videoOffsetX.U &&
        dpiX < (videoOffsetX + (videoWidth * videoScale)).U &&
        dpiY >= videoOffsetY.U &&
        dpiY < (videoOffsetY + (videoHeight * videoScale)).U

      // Scale overlay
      val overlayScale = 2
      val overlayReadDelay = 3
      overlayReadAddress :=
        ((((dpiY + overlayReadDelay.U) / overlayScale.U) + overlayYControl.scroll)(7, 0) * overlayWidth.U) +
          ((dpiX / overlayScale.U) + overlayXControl.scroll)(7, 0)
      overlayInBounds := videoX >= (overlayXControl.start * overlayScale.U) &&
        videoX < (overlayXControl.end * overlayScale.U) &&
        videoY >= (overlayYControl.start * overlayScale.U) &&
        videoY < (overlayYControl.end * overlayScale.U)
    }
  }

//  io.pmod.dir := "b1111".U
//  io.pmod.out := io.lcd.asUInt
//  module.io.pmod.in := 0.U

  // Overlay access.
  // TODO: consider switching to (or adding) a method of writing where
  // there's a "target x" and "target y" register, and you write to a single
  // memory location, which auto-increments the x. Then, have registers for
  // minX (where it wraps to) and maxX (when it wraps), which allows for easy
  // partial rectangular updates.
  overlayInterface.dataRead := DontCare
  overlayInterface.done := false.B
  overlayFramebuffer.writePorts(0).enable := overlayInterface.enable && overlayInterface.write
  overlayFramebuffer.writePorts(0).address := (overlayInterface.address >> 1).asUInt
  overlayFramebuffer.writePorts(0).data := overlayInterface.dataWrite
  overlayInterface.done := RegNext(overlayInterface.enable)

  // Framebuffer read via SPI.
  for (i <- 0 until 3) {
    framebuffers(i).readwritePorts(0).enable := false.B
    framebuffers(i).readwritePorts(0).address := DontCare
    framebuffers(i).readwritePorts(0).isWrite := DontCare
    framebuffers(i).readwritePorts(0).writeData := DontCare
  }
  val framebufferInterfaceRead = framebufferInterface.enable && !framebufferInterface.write
  when (framebufferInterfaceRead) {
    for (i <- 0 until 3) {
      when (framebufferControl.io.latestIndex === i.U) {
        framebuffers(i).readwritePorts(0).enable := true.B
        framebuffers(i).readwritePorts(0).address := (framebufferInterface.address >> 1.U).asUInt
        framebuffers(i).readwritePorts(0).isWrite := false.B
      }
    }
  }
  framebufferInterface.dataRead := MuxLookup(framebufferControl.io.latestIndex, 0.U)(
    (0 until 3).map(i => i.U ->
      RegNext(RegNext(framebuffers(i).readwritePorts(0).readData))
    ))
  framebufferInterface.done := RegNext(RegNext(framebufferInterface.enable))

  //////////////////////////////////
  // Submodule Connections
  //////////////////////////////////
  module.io.enable := controlRegister.moduleEnable
  module.io.reset := !controlRegister.moduleReset
  io.vibrate := (module.io.enable && module.io.vibrate) && controlRegister.vibrate
  io.link <> module.io.link
  io.pmod <> module.io.pmod
  module.io.mcuInterface <> moduleMcuInterface

  // Buttons must be synchronized and inverted.
  module.io.buttons :=
    (RegNext(RegNext(~io.buttons.asUInt)).asUInt | buttonRegister.asUInt).asTypeOf(new HandheldButtons)

  // Framebuffer writes
  when (module.io.framebufferWriteEnable && !framebufferInterfaceRead) {
    // Module framebuffer write and SPI framebuffer read share the same read/write port,
    // so ensure that they're not activated at the same time (so they can be inferred correctly).
    val address = (module.io.framebufferY * videoWidth.U(8.W)) + module.io.framebufferX
    for (i <- 0 until 3) {
      framebuffers(i).readwritePorts(0).enable := (i.U === framebufferControl.io.writeIndex)
      framebuffers(i).readwritePorts(0).address := address
      framebuffers(i).readwritePorts(0).isWrite := true.B
      framebuffers(i).readwritePorts(0).writeData := module.io.framebufferData.asUInt
    }
  }
  framebufferControl.io.writeActive := !module.io.vblank

  // N.B. Audio synchronization happens above.

  // Cartridge
  io.cartridge <> module.io.cartridge
  io.cartridgeOutputEnableN := !module.io.cartridgeEnabled
  io.cartridge3V3Enable := !io.cartridgeSwitch
  io.cartridge5V0Enable := io.cartridgeSwitch

  // Memories
  sramArbiter.io.initiator(1) <> module.io.sram
  sdramArbiter.io.initiator(1) <> module.io.sdram
}
