package platform.handheld

import chisel3._
import chiseltest._
import org.scalatest.freespec.AnyFreeSpec

class I2sTransmitterSpec extends AnyFreeSpec with ChiselScalatestTester {
  "go" in {
    test(new I2sTransmitter(
      bitWidth = 16,
      mclkFactor = 256,
      channels = 2,
    )) { dut =>
      dut.io.dataLeft.poke(0xFFFF)
      dut.io.dataRight.poke(0xAAAA)
      for (i <- 0 until 2) {
        for (j <- 0 until 256) {
          if (dut.io.signals.mclk.peekBoolean()) {
            print("M")
          } else {
            print(" ")
          }
          if (dut.io.signals.wclk.peekBoolean()) {
            print("W")
          } else {
            print(" ")
          }
          if (dut.io.signals.bclk.peekBoolean()) {
            print("B")
          } else {
            print(" ")
          }
          if (dut.io.signals.data.peekInt() == 1) {
            print("*")
          } else {
            print(" ")
          }
          println()
          dut.clock.step()
        }
        println("------------------------------------------")
      }
    }
  }
}
