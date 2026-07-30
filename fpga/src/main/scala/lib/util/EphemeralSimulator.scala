package lib.util

import svsim._
import chisel3.RawModule
import chisel3.simulator._
import java.nio.file.Files
import java.io.File
import scala.reflect.io.Directory

/// Based off of chisel3.simulator.EphemeralSimulator, with additional Verilator options
object EphemeralSimulator extends PeekPokeAPI {

  def simulate[T <: RawModule](
                                module: => T
                              )(body:   (T) => Unit
                              ): Unit = {
    makeSimulator.simulate(module)({ module => body(module.wrapped) }).result
  }

  private class DefaultSimulator(val workspacePath: String) extends SingleBackendSimulator[verilator.Backend] {
    val backend = verilator.Backend.initializeFromProcessEnvironment()
    val tag = "default"
    val commonCompilationSettings = CommonCompilationSettings()
    val backendSpecificCompilationSettings = verilator.Backend.CompilationSettings(
      disabledWarnings = Seq("WIDTHEXPAND")
    )

    // Try to clean up temporary workspace if possible
    sys.addShutdownHook {
      (new Directory(new File(workspacePath))).deleteRecursively()
    }
  }
  private def makeSimulator: DefaultSimulator = {
    val id = ProcessHandle.current().pid().toString()
    val className = getClass().getName().stripSuffix("$")
    new DefaultSimulator(Files.createTempDirectory(s"${className}_${id}_").toString)
  }
}
