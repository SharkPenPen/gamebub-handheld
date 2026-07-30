# Simulator

Most development can be done in a Verilator simulation for easier debugging and to avoid having to build a new bitstream for every change.

Note that while the simulator is fast to compile, it's very slow to run. Only single thread CPU performance matters. On a MacBook Air M3, the GBA simulator runs at  ~5 fps (real time is 60fps), and the Game Boy Color simulator runs at ~40 fps.

The simulator has only been tested on macOS, but Linux should work perfectly fine as well.

## Setup

Ensure this repo is cloned with submodules checked out. If you already cloned the repo, run this:

```
git submodule update --init --recursive
```

SDL2 and [Verilator](https://www.veripool.org/verilator/) must be installed.
`fusesoc` and `sbt` (Scala Build Tool) must also be installed, see [building.md](./building.md).


## Building

To build the Game Boy simulator, run:

```
fusesoc --cores-root . run --target=sim --flag=module_gameboy elipsitz:gameboy:gameboy
```

The simulator will be built at `./build/elipsitz_gameboy_gameboy_1.0.0/sim-verilator/VSimGameboy`

Similarly, for the GBA, run:

```
fusesoc --cores-root . run --target=sim --flag=module_gba elipsitz:gameboy:gameboy
```

And find the simulator at `./build/elipsitz_gameboy_gameboy_1.0.0/sim-verilator/VSimGba`

Note that this command will end with an error ("ERROR: Failed to run ...") -- that's okay! FuseSoC doesn't know how to provide the correct command-line arguments to run the simulator.


## Running

For Game Boy Advance:

```
./build/elipsitz_gameboy_gameboy_1.0.0/sim-verilator/VSimGba --bios-path <path to bios.bin> <path to rom.gba>
```

Game Boy is the same, but with `VSimGameboy` (as mentioned above).

Optionally pass `--record path/to/output.mp4` to record all of the output of the simulator to play it back at full speed. Note that this produces an unusual lossless `mp4` file, and not all video players can play it. VLC works, though.

### Controls

Simulator controls:
* `escape`: quit the simulator
* `cmd+p`: pause or unpause
* `cmd+n`: pause and enter frame-by-frame mode. Keep pressing to advance one frame at a time.
* `cmd+r`: reset the simulator

Game controls:
* Arrow keys: D-pad up/down/left/right
* `z`: A button
* `x`: B button
* `a`: L shoulder button
* `s`: R shoulder button
* `right shift`: Select button
* `return`: Start button
