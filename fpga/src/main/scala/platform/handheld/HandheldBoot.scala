package platform.handheld

import chisel3._
import chisel3.util._
import lib.mem.RegisterMap
import lib.video.ColorARGB

import javax.imageio.ImageIO
import java.awt.Color

class AnimationState extends Bundle {
    val time = UInt(8.W)
    val loop = Bool()
    val running = Bool()
}

class HandheldBoot extends Module with HandheldModule {
    val io = IO(new HandheldIo)
    def framebufferW = 240
    def framebufferH = 160
    def clockSystemHz = 8 * 1024 * 1024
    def clockSdramHz = clockSystemHz * 4
    stubUnused()

    val regAnimation = RegInit(0.U.asTypeOf(new AnimationState))
    val regLogoStartY = RegInit(26.U(8.W))

    val regX = RegInit(0.U(log2Ceil(framebufferW).W))
    val regY = RegInit(0.U(log2Ceil(framebufferH).W))

    val (_, frame) = Counter(true.B, clockSystemHz / 64)

    io.mcuInterface <> RegisterMap(
        addressWidth = 8,
        dataWidth = 32,
        entries = Seq(
            0x0 -> RegisterMap.Entry.rw(regAnimation),
            0x4 -> RegisterMap.Entry.rw(regLogoStartY),
        )
    )

    // Load and process logo
    val (logoW, logoH, logoData) = loadLogo()
    val logo = VecInit(logoData.map(x => x.U(8.W)))
    val bgColor = Wire(ColorARGB.rgb555())
    bgColor.a := 0.U
    bgColor.r := (0xE8 >> 3).U(5.W)
    bgColor.g := (0xE8 >> 3).U(5.W)
    bgColor.b := (0xE8 >> 3).U(5.W)
    val logoStartX = (framebufferW - logoW) / 2
    val logoEndX = (framebufferW + logoW) / 2
    val colorTable = makeColorTable(logoW)
    val colorOffX = RegInit(0.U(log2Ceil(3 * logoW).W))

    when (frame) {
        regX := 0.U
        regY := 0.U

        when (regAnimation.running) {
            val nextTime = regAnimation.time + 4.U
            when ((nextTime < regAnimation.time) && !regAnimation.loop) {
                regAnimation.time := 0.U
                regAnimation.running := false.B
            } .otherwise {
                regAnimation.time := nextTime
            }
        }

        // TODO calculate colorOffX with the curve
        colorOffX := ((regAnimation.time * (logoW * 2).U) >> 8).asUInt
    } .otherwise {
        when (regY === (framebufferH - 1).U) {
            when (regX < framebufferW.U) {
                regY := 0.U
                regX := regX + 1.U
            }
        } .otherwise {
            regY := regY + 1.U
        }
    }
    io.framebufferX := regX
    io.framebufferY := regY
    io.framebufferWriteEnable := regX < framebufferW.U
    io.vblank := !io.framebufferWriteEnable
    io.framebufferData.a := DontCare

    io.framebufferData.r := bgColor.r
    io.framebufferData.g := bgColor.g
    io.framebufferData.b := bgColor.b
    when (regX >= logoStartX.U && regX < logoEndX.U && regY >= regLogoStartY && regY < (regLogoStartY + logoH.U)) {
        // TODO read one or two early
        val x = regX - logoStartX.U
        val y = regY - regLogoStartY
        val alpha = logo((x * logoH.U + y)(log2Ceil(logoData.length) - 1, 0))

        val colorX = colorOffX + x
        val color = WireDefault(colorTable(0))
        when (colorX >= logoW.U && colorX < (logoW * 2).U) {
            color := colorTable(colorX - logoW.U)
        }

        // Alpha blend: out = (A * alpha) + (1 - alpha) * B
        // Division by 256 will be slightly off, because the alpha is between 0 and 255.
        io.framebufferData.r := blend(color.r, bgColor.r, alpha)
        io.framebufferData.g := blend(color.g, bgColor.g, alpha)
        io.framebufferData.b := blend(color.b, bgColor.b, alpha)
    }

    private def blend(a: UInt, b: UInt, alpha: UInt): UInt = {
        (((a * alpha) + ((0x20.U(6.W) - alpha) * b)) >> 5).asUInt
    }

    private def loadLogo(): (Int, Int, Seq[Int]) = {
        val logo = ImageIO.read(getClass.getClassLoader.getResource("logo.png"))
        val data = (0 until (logo.getWidth * logo.getHeight)).map(i => {
            val x = i / logo.getHeight
            val y = i % logo.getHeight
            val alpha = (logo.getRGB(x, y) >> 24) & 0xFF
            alpha >> 3 // convert to 5 bit color
        })
        (logo.getWidth, logo.getHeight, data)
    }

    /// Make the gradient color table
    private def makeColorTable(width: Int) = {
        VecInit((0 until width).map(x => {
            val t = x.toDouble / (width - 1)
            val hueDelta = (-4.0 * t * (t - 1.0)) * 0.2 // TODO: or 0.15?

            val hsbVals = Color.RGBtoHSB(0x3A, 0x3C, 0x99, null)
            hsbVals(0) += hueDelta.toFloat
            val out = Color.getHSBColor(hsbVals(0), hsbVals(1), hsbVals(2))
            val color = Wire(ColorARGB.rgb555())
            color.a := DontCare
            color.r := (out.getRed >> 3).U(5.W)
            color.g := (out.getGreen >> 3).U(5.W)
            color.b := (out.getBlue >> 3).U(5.W)
            color
        }))
    }

    private def stubUnused(): Unit = {
        io.vibrate := false.B
        io.audioLeft := 0.S
        io.audioRight := 0.S

        // Cartridge unused
        io.cartridgeEnabled := false.B
        io.cartridge.bank0Dir := false.B
        io.cartridge.bank1Dir := false.B
        io.cartridge.bank2Dir := false.B
        io.cartridge.bank3Dir := false.B
        io.cartridge.pin30Dir := false.B
        io.cartridge.pin31Dir := false.B
        io.cartridge.bank0Out := DontCare
        io.cartridge.bank1Out := DontCare
        io.cartridge.bank2Out := DontCare
        io.cartridge.bank3Out := DontCare
        io.cartridge.pin30Out := DontCare
        io.cartridge.pin31Out := DontCare

        // PMOD unused
        io.pmod.out := DontCare
        io.pmod.dir := 0.U(4.W)

        // Link unused
        io.link.soOut := DontCare
        io.link.siOut := DontCare
        io.link.sdOut := DontCare
        io.link.scOut := DontCare
        io.link.soDir := false.B
        io.link.siDir := false.B
        io.link.sdDir := false.B
        io.link.scDir := false.B

        // SRAM unused
        io.sram.enable := false.B
        io.sram.write := false.B
        io.sram.address := DontCare
        io.sram.dataWrite := DontCare
        io.sram.writeStrobe := DontCare

        // SDRAM unused
        io.sdram.enable := false.B
        io.sdram.write := false.B
        io.sdram.address := DontCare
        io.sdram.dataWrite := DontCare
        io.sdram.writeStrobe := DontCare
    }
}