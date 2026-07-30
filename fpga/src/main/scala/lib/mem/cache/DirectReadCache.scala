package lib.mem.cache

import chisel3._
import chisel3.util._
import lib.mem.MemoryInterface
import lib.mem.cache.DirectReadCache._

object DirectReadCache {
  object State extends ChiselEnum {
    val init, idle, waitCache, waitMem, doneMem = Value
  }

  class Entry(tagWidth: Int, dataWidth: Int) extends Bundle {
    val valid = Bool()
    val tag = UInt(tagWidth.W)
    val data = UInt(dataWidth.W)
  }
}


/*
 * A simple, direct-mapped, read-only cache.
 *
 * Writes are passed through, but not cached.
 */
class DirectReadCache(addressWidth: Int, dataWidth: Int, numEntries: Int) extends Module {
  val io = IO(new Bundle {
    val in = new MemoryInterface(addressWidth, dataWidth)
    val out = Flipped(new MemoryInterface(addressWidth, dataWidth))
  })
  assert(isPow2(numEntries))
  val indexWidth = log2Ceil(numEntries)
  val tagWidth = addressWidth - indexWidth

  val entryType = new Entry(tagWidth, dataWidth)
  val cache = SRAM(numEntries, UInt(entryType.getWidth.W), numReadPorts = 0, numWritePorts = 0, numReadwritePorts = 1)
  val state = RegInit(State.init)
  val initIndex = RegInit(0.U(indexWidth.W))

  val cachePort = cache.readwritePorts(0)
  cachePort.enable := false.B
  cachePort.address := DontCare
  cachePort.isWrite := DontCare
  cachePort.writeData := DontCare

  /*
   * TODO: this regAddress shouldn't actually be needed:
   *  it's just to avoid a combinatorial loop in HandheldGba.
   */
  val regAddress = Reg(UInt(addressWidth.W))
  val regDataRead = Reg(UInt(dataWidth.W))
  io.in.done := false.B
  io.in.dataRead := DontCare
  io.out.enable := false.B
  io.out.address := regAddress
  io.out.write := io.in.write
  io.out.dataWrite := io.in.dataWrite
  io.out.writeStrobe := io.in.writeStrobe

  private def getIndex(address: UInt): UInt = address(indexWidth - 1, 0)
  private def getTag(address: UInt): UInt = address(addressWidth - 1, indexWidth)

  switch (state) {
    // Initialization: upon reset, iterate through the cache and invalidate each entry.
    is (State.init) {
      val nextInitIndex = initIndex + 1.U
      cachePort.enable := true.B
      cachePort.isWrite := true.B
      cachePort.address := initIndex
      cachePort.writeData := 0.U
      initIndex := nextInitIndex
      when (nextInitIndex === 0.U) {
        state := State.idle
      }
    }

    is (State.idle) {
      when (io.in.enable) {
        // Check the cache for the data.
        // TODO: invalidate on writes
        cachePort.enable := true.B
        cachePort.isWrite := false.B
        cachePort.address := getIndex(io.in.address)
        state := State.waitCache
        regAddress := io.in.address
      }
    }

    is (State.waitCache) {
      val entry = cachePort.readData.asTypeOf(entryType)
      when (entry.valid && entry.tag === getTag(regAddress)) {
        // Cache hit!
        io.in.done := true.B
        io.in.dataRead := entry.data
        state := State.idle
      } .otherwise {
        // Cache miss, begin the read of main memory.
        io.out.enable := true.B
        state := State.waitMem
      }
    }

    is (State.waitMem) {
      io.out.enable := true.B
      when (io.out.done) {
        // Insert into cache.
        cachePort.enable := true.B
        cachePort.isWrite := true.B
        cachePort.address := getIndex(regAddress)
        val wire = Wire(entryType)
        wire.valid := true.B
        wire.data := io.out.dataRead
        wire.tag := getTag(regAddress)
        cachePort.writeData := wire.asUInt

        // We don't set io.in.done = true and return the data this cycle, because the 'done' signal might be
        // coming from a faster clock domain. If we forward it directly here, then the downstream consumer
        // will only have a *fast* clock period to do all logic.
        // Instead, register it and pass it forward next cycle.
        // TODO this should almost certainly be happening at a layer above this
        regDataRead := io.out.dataRead
        state := State.doneMem
      }
    }

    is (State.doneMem) {
      io.in.done := true.B
      io.in.dataRead := regDataRead
      state := State.idle
    }
  }
}
