package platform.handheld

import chisel3._

class TripleBufferControl extends Module {
  val io = IO(new Bundle {
    // Whether the read and write phases are currently active.
    val readActive = Input(Bool())
    val writeActive = Input(Bool())

    // Read and write buffer indexes.
    // Only valid when the corresponding "active" is true (starting the next cycle after active goes high).
    val readIndex = Output(UInt(2.W))
    val writeIndex = Output(UInt(2.W))

    // Most "up to date" buffer -- most recently written with complete data.
    val latestIndex = Output(UInt(2.W))

    // Number of duplicated read frames (i.e. write is too slow)
    val statNumDuplicated = Output(UInt(24.W))
    // Number of skipped frames (i.e. read is too slow)
    val statNumSkipped = Output(UInt(24.W))
  })

  /// Indexes of the read, write, and latest buffer.
  val writeIndex = RegInit(0.U(2.W))
  val readIndex = RegInit(1.U(2.W))
  val latestIndex = RegInit(2.U(2.W))
  io.writeIndex := writeIndex
  io.readIndex := readIndex
  io.latestIndex := latestIndex

  val statNumDuplicated = RegInit(0.U(24.W))
  val statNumSkipped = RegInit(0.U(24.W))
  io.statNumDuplicated := statNumDuplicated
  io.statNumSkipped := statNumSkipped

  // Read/write start/end
  val prevWriteActive = RegNext(io.writeActive, false.B)
  val writeStart = io.writeActive && !prevWriteActive
  val writeEnd = !io.writeActive && prevWriteActive
  val prevReadActive = RegNext(io.readActive, false.B)
  val readStart = io.readActive && !prevReadActive
  val readEnd = !io.readActive && prevReadActive

  when (readStart && writeEnd) {
    // Special case: both updated at the same time.
    readIndex := writeIndex
    // Arbitrary: could be the other buffer too.
    writeIndex := readIndex
    latestIndex := writeIndex
  } .otherwise {
    when (writeEnd) {
      // A buffer was just written.
      // Set the "latest complete buffer" to the one that was just completed.
      latestIndex := writeIndex
      writeIndex := (0 + 1 + 2).U - readIndex - writeIndex

      when (readIndex =/= latestIndex) {
        // We overwrote the "latest" index before the read index actually got to it,
        // so a frame was skipped.
        statNumSkipped := statNumSkipped + 1.U
      }
    }
    when (readStart) {
      // The buffer is about to start reading out, switch to the latest available.
      readIndex := latestIndex

      when (readIndex === latestIndex) {
        // We're reading the same one we just read, so a frame will be duplicated.
        statNumDuplicated := statNumDuplicated + 1.U
      }
    }
  }

  // TODO: a more complex method where we keep track of "free" buffers.
  // A buffer is acquired when write or read starts, and released when write or read ends.
  // And some way of transferring the most recent buffer from write to read.
  // This reduces some possible frame repetition since there's a vblank period on both sides.
}
