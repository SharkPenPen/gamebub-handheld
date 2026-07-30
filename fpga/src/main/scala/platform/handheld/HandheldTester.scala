package platform.handheld;

import chisel3._
import chisel3.util._
import lib.mem.RegisterMap

class HandheldTester extends Module with HandheldModule {
    val io = IO(new HandheldIo)
    def framebufferW = 240
    def framebufferH = 160
    def clockSystemHz = 8 * 1024 * 1024
    def clockSdramHz = clockSystemHz * 4

    val (_, frame) = Counter(true.B, clockSystemHz / 60)

    val offX = RegInit(0.U(5.W))
    val offY = RegInit(0.U(5.W))
    val x = RegInit(0.U(8.W))
    val y = RegInit(0.U(8.W))

    io.mcuInterface <> RegisterMap(
        addressWidth = 8,
        dataWidth = 32,
        entries = Seq(
            0x0 -> RegisterMap.Entry.rw(offX),
            0x4 -> RegisterMap.Entry.rw(offY),
        )
    )

    when (frame) {
        x := 0.U
        y := 0.U
    } .otherwise {
        when (x === (framebufferW - 1).U) {
            when (y < framebufferH.U) {
                x := 0.U
                y := y + 1.U
            }
        } .otherwise {
            x := x + 1.U
        }
    }
    io.framebufferX := x
    io.framebufferY := y
    io.framebufferWriteEnable := y < framebufferH.U
    io.vblank := !io.framebufferWriteEnable
    io.framebufferData.a := DontCare
    io.framebufferData.r := 0.U
    io.framebufferData.g := 0.U
    io.framebufferData.b := 0.U

    // Generate test video pattern
    when (io.buttons.start) {
        // Border test pattern
        when (x === 0.U && y === 0.U) {
            // Top-left corner: red
            io.framebufferData.r := 0x1F.U
        } .elsewhen ((x === 0.U && y === 159.U) || (x === 239.U && y === 0.U) || (x === 239.U && y === 159.U)) {
            // Other corners: yellow
            io.framebufferData.r := 0x1F.U
            io.framebufferData.g := 0x1F.U
        } .elsewhen (x === 0.U || x === 239.U || y === 0.U || y === 159.U) {
            // Outer border: green
            io.framebufferData.g := 0x1F.U
        } .elsewhen (x === 1.U || x === 238.U || y === 1.U || y === 158.U) {
            // Next border: blue
            io.framebufferData.b := 0x1F.U
        } .otherwise {
            // Middle: gray
            io.framebufferData.r := 0xF.U
            io.framebufferData.g := 0xF.U
            io.framebufferData.b := 0xF.U
        }
    } .otherwise {
        // XOR test pattern -- moves to left
        // X and Y are flipped: columns are R, G, B
        val color = (offX + x)(4, 0) ^ (offY + y)(4, 0)
        when (x < 80.U) {
            io.framebufferData.r := color
        } .elsewhen(x < 160.U) {
            io.framebufferData.g := color
        } .otherwise {
            io.framebufferData.b := color
        }
    }


    // Movement
    when (io.enable && frame) {
        val speed = Mux(io.buttons.a, 2.U, 1.U)
        when (io.buttons.left) {
            offX := offX - speed
        }
        when (io.buttons.right) {
            offX := offX + speed
        }
        when (io.buttons.up) {
            offY := offY - speed
        }
        when (io.buttons.down) {
            offY := offY + speed
        }
    }

    // Audio
    val audioData = RegInit(10_000.S(16.W))
    val sampleCounter = Counter(clockSystemHz / 440 / 2)
    when (sampleCounter.inc()) {
        audioData := -audioData
    }
    when (io.buttons.x) {
        io.audioLeft := audioData
        io.audioRight := audioData
    } .otherwise {
        io.audioLeft := 0.S
        io.audioRight := 0.S
    }

    io.vibrate := io.buttons.l

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