package platform.handheld

import chisel3._
import chisel3.util._
import lib.mem.MemoryInterface
import platform.handheld.AsyncSramController.State

object AsyncSramController {
  object State extends ChiselEnum {
    val idle, write, read = Value
  }

  class Signals(addressWidth: Int, dataWidth: Int) extends Bundle {
    val address = Output(UInt(addressWidth.W))
    val dataIn = Input(UInt(dataWidth.W))
    val dataOut = Output(UInt(dataWidth.W))
    val dataDir = Output(Bool())
    val oeN = Output(Bool())
    val weN = Output(Bool())
    val writeMaskN = Output(UInt((dataWidth / 8).W))
  }
}

/**
 * A controller for a single asynchronous SRAM chip.
 *
 * nCE is assumed to always be low.
 * Memory interface is word addressed and has byte strobe.
 */
class AsyncSramController(addressWidth: Int, dataWidth: Int) extends Module {
  val io = IO(new Bundle {
    val signals = new AsyncSramController.Signals(addressWidth, dataWidth)

    /** Standard memory interface to consumers. */
    val mem = new MemoryInterface(addressWidth, dataWidth)
  })
  val maskWidth = dataWidth / 8

  val state = RegInit(State.idle)
  val regDone = RegInit(false.B)
  val regAddress = Reg(UInt(addressWidth.W))
  val regDataOut = Reg(UInt(dataWidth.W))
  val regDataIn = Reg(UInt(dataWidth.W))
  val regDataDir = RegInit(false.B)
  val regWriteMaskN = RegInit(0.U(maskWidth.W))
  val regOeN = RegInit(true.B)
  val regWeN = RegInit(true.B)

  io.signals.address := regAddress
  io.signals.dataOut := regDataOut
  io.signals.writeMaskN := regWriteMaskN
  io.signals.dataDir := regDataDir
  io.signals.oeN := regOeN
  io.signals.weN := regWeN
  io.mem.dataRead := regDataIn
  io.mem.done := regDone

  switch (state) {
    is (State.idle) {
      regDone := false.B

      when (io.mem.enable && !regDone) {
        regAddress := io.mem.address

        when (io.mem.write) {
          state := State.write
          regDataOut := io.mem.dataWrite
          regWriteMaskN := ~io.mem.writeStrobe
          regWeN := false.B
          regDataDir := true.B
        } .otherwise {
          state := State.read
          regOeN := false.B
        }
      }
    }
    is (State.read) {
      state := State.idle
      regDone := true.B
      regOeN := true.B
      regDataIn := io.signals.dataIn
    }
    is (State.write) {
      state := State.idle
      regDone := true.B
      regWeN := true.B
      regDataDir := false.B
      regWriteMaskN := 0.U
    }
  }
}
