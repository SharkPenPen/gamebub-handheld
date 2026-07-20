package platform.handheld

import chisel3._
import chisel3.util._

class I2sSignals extends Bundle {
  /** Master Clock */
  val mclk = Output(Bool())
  /** Word Clock */
  val wclk = Output(Bool())
  /** Bit Clock */
  val bclk = Output(Bool())
  /** Serial Data */
  val data = Output(UInt(1.W))
}

class I2sTransmitter(
  bitWidth: Int = 16,
  mclkFactor: Int = 256, // MCLK = clockMul * sampleRate
  channels: Int = 2,
) extends Module {
  val io = IO(new Bundle {
    val signals = Output(new I2sSignals)

    /** Whether the audio data is being sampled this cycle. */
    val sampleEnable = Output(Bool())
    /** The audio data for the left channel. */
    val dataLeft = Input(UInt(bitWidth.W))
    /** The audio data for the right channel. */
    val dataRight = Input(UInt(bitWidth.W))
  })
  assert(channels == 2)

  val counter = Counter(mclkFactor)
  io.sampleEnable := counter.inc()
  val sampleLeft = RegInit(0.U(bitWidth.W))
  val sampleRight = RegInit(0.U(bitWidth.W))
  when (io.sampleEnable) {
    sampleLeft := io.dataLeft
    sampleRight := io.dataRight
  }

  io.signals.mclk := clock.asBool
  // Word clock is MSB of counter
  io.signals.wclk := counter.value(log2Ceil(mclkFactor) - 1)
  // Aiming for double the bits per word:
  //   2 (clock sides) * 16 (bit width) * 2 (double per channel) * 2 (channels) = 128 --
  //   so take the 1st lower bit of the counter.
  //   TODO: make this change with the parameters
  assert(mclkFactor == 256)
  assert(bitWidth == 16)
  io.signals.bclk := counter.value(1)


  // "In I2S mode, the MSB of the left channel is valid on the second rising edge
  // of the bit clock after the falling edge of the word clock."
  // Out register is padded to ensure this happens.
  val outRegister = RegInit(0.U((bitWidth + 1).W))
  io.signals.data := outRegister(bitWidth)
  when (counter.value === 0.U) {
    // Load the left channel sample
    outRegister := Cat(0.U(1.W), sampleLeft)
  } .elsewhen (counter.value === (mclkFactor / 2).U) {
    // Load the right channel sample
    outRegister := Cat(0.U(1.W), sampleRight)
  } .elsewhen (counter.value(1, 0) === 0.U) {
    outRegister := outRegister << 1
//    printf(cf"shifting, counter = ${counter.value}\n")
  }
}
