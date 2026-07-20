# Accuracy

Neither the Game Boy [Color] or Game Boy Advance emulators have any major issues in any games I've tested.

## Game Boy / Game Boy Color

Test results WIP

## Game Boy Advance

Summary of test results (using [tests from here](https://emulation.gametechwiki.com/index.php/GBA_Tests)), as of commit [7d47149](/../../tree/7d47149a2f52bd7607accf401fc958f4a95d7e3a).

#### [mGBA test suite](https://github.com/mgba-emu/suite), commit 0293d27

| Test name  | Total | Passed |
| ---------  | ----- | ------ |
| Memory tests | 1552 | 1356 |
| I/O read tests | 130 | 43 |
| Timing tests | 2020 | 1510 |
| Timer count-up tests | 936 | 603 |
| Timer IRQ tests | 90 | 45 |
| Shifter tests | 140 | **140** |
| Carry tests | 93 | **93** |
| Multiply long tests | 72 | 52 |
| BIOS math tests | 615 | **615** |
| DMA tests | 1256 | 1060 |
| SIO register R/W tests | 90 | 32 |
| SIO timing tests | 8 | 0 |
| Misc. edge case tests | 10 | 8 |
| Video tests  | 7 | 5 |

#### AGS Aging Cartridge v10.0

| Test name  | Total | Passed |
| ---------  | ----- | ------ |
| Memory | 9 | **9** |
| LCD | 7 | **7** |
| Timer | 3 | **3** |
| DMA | 9 | **9** |
| COM | N/A | N/A |
| Key Input | 1 | **1** |
| Interrupt | 7 | **7** |

#### [jsmolka gba-tests](https://github.com/jsmolka/gba-tests), commit 49204e1

| Test name  | Result |
| ---------  | ------ |
| arm | **Pass** |
| bios | **Pass** |
| memory | **Pass** |
| nes | **Pass** |
| thumb | **Pass** |
| save: none | **Pass** |
| save: sram | **Pass** |
| save: flash64 | **Pass** |
| save: flash128 | **Pass** |

#### [alyosha gba-tests](https://github.com/alyosha-tas/gba-tests/), commit c8e15ff

| Test name  | Result |
| ---------  | ------ |
| DMA_ROM_Fixed | Fail test 46 |
| FIFO | Fail test 1 |
| FIFO_2 | Fail test 78 |
| Internal_Cycle_DMA_IRQ | Fail test 6 |
| Internal_Cycle_DMA_Mul | Fail test 37 |
| Halt_PC | Fail test 212 |
| Halt_PC_2 | Fail test 236 |
| Halt_PC_3 | Fail test 233 |
| Halt_PC_4 | Fail test 239 |
| IE | Fail test 1 |
| IF | Fail test 1 |
| IF_Timer | Fail test 1 |
| IRQ_Sub | **Pass** |
| LDM_ALU | Fail test 2 |
| LDM_ALU_Store | Fail test 6 |
| Sprite disable midline | not tested |
| Sprite disable on VRAM access | not tested |
| Sprite Last VRAM access | Fail test 159 |
| Sprite Last VRAM access free |  Fail test 121 |
| boundary_test_1 | Fail test 34 |
| branch_thumb | Fail test 42 |
| branch_thumb_2 | Fail test 26 |
| branch_thumb_3 | Fail test 34 |
| branch_thumb_4 | Fail test 29 |
| branch_thumb_arm | Fail test 55 |
| prefetcher_dma | Fail test 52 |
| prefetcher_full_arm | Fail test 1 |
| prefetcher_full_thumb | fail test 49 |
| PSR | Fail test 4 |
| Timer | Fail test 1 |
| Timer disable | Fail test 244 |
| Timer reset | Fail test 1 |

#### [NBA hw-test](https://github.com/nba-emu/hw-test), commit 4571fc5

| Test name  | Result |
| ---------  | ------ |
| Bus 128kb-boundary | Fail (stuck?) |
| DMA burst-into-tears | Fail 0/3 |
| DMA force-nseq-access | Fail 0/2 |
| DMA latch | Fail 0/3 |
| DMA start-delay | Fail 0/1 |
| IRQ irq-delay | **Pass** |
| PPU bgpd | Fail |
| PPU bgx | Fail |
| PPU dispcnt-latch | Fail |
| PPU greenswap | Fail |
| PPU sprite-hmosaic | Fail |
| PPU status-irq-dma | Fail (stuck?) |
| PPU vram-mirror | Fail 7/10 |
| Timer Reload | **Pass** |
| Timer Start-stop | **Pass** |
| Timer Halt-cnt | Fail 1/6 |

#### [Hades-Tests](https://github.com/hades-emu/Hades-Tests), commit 25ec4b4

| Test name  | Result |
| ---------  | ------ |
| bios-openbus | **Pass 12/12** |
| dma-latch | Fail 0/4 |
| dma-start-delay | Fail 0/8 |
| timer-basic | **Pass 10/10** |

#### [AGBEEG Aging v0.0.2](https://github.com/zaydlang/AGBEEG-Aging-Cartridge)

| Test name  | Result |
| ---------  | ------ |
| Cartridge | Fail 0/2 |
| CPU | Fail 2/3 |
| DMA | Fail 0/1 |
