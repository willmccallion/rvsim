#include "bench.h"
#include "stdio.h"

// Demonstrates EX->EX forwarding vs load-use stalls.
//
// Phase 1 (ALU chain): each iteration chains ADD results through forwarding.
//   The pipeline can forward EX-stage results directly to the next instruction's
//   EX stage, so no stall bubbles are inserted. Expect ~1 cycle/iter throughput.
//
// Phase 2 (load-use): a LOAD is immediately followed by an instruction that
//   consumes the loaded value. Because the load result is not available until
//   the end of MEM, the pipeline must insert a 1-cycle stall bubble before the
//   dependent ALU instruction. Expect ~2 cycles/iter throughput.
//
// The cycle ratio (warm/alu) should be close to 2.0 on an un-forwarded pipeline
// and closer to 1.5-2.0 depending on forwarding implementation.

volatile long a = 1, b = 2, c = 3; // ALU chain operands
volatile long val = 42;             // source for load-use chain

int main() {
    long tmp;
    unsigned long start, end;

    // ------------------------------------------------------------------
    // Phase 1: ALU chain — EX->EX forwarding, no stalls expected
    // ------------------------------------------------------------------
    // a = b + c  ->  c = a + b  ->  a = b + c  ...
    // Each result is forwarded directly from EX to the next EX stage.
    start = read_cycles();
    for (int i = 0; i < 5000; i++) {
        a = b + c; // produces 'a', forwarded to next instruction
        c = a + b; // consumes 'a' via EX->EX forward path
    }
    end = read_cycles();
    unsigned long alu_cycles = end - start;

    // ------------------------------------------------------------------
    // Phase 2: load-use — LD then immediate use, stall expected
    // ------------------------------------------------------------------
    // Each iteration loads 'val' from memory then immediately uses the
    // result in an ADD. The pipeline cannot forward from MEM/WB to EX
    // without a 1-cycle stall bubble (load-use hazard).
    start = read_cycles();
    for (int i = 0; i < 5000; i++) {
        tmp = val;     // LOAD — result not available until end of MEM
        a = tmp + 1;   // must wait one extra cycle for load result
    }
    end = read_cycles();
    unsigned long load_use_cycles = end - start;

    printf("Phase 1 ALU chain  (5000 iters): %lu cycles\n", alu_cycles);
    printf("Phase 2 Load-use   (5000 iters): %lu cycles\n", load_use_cycles);

    // Print integer ratio as 10x fixed-point to avoid floating point
    unsigned long ratio10 = (load_use_cycles * 10) / (alu_cycles ? alu_cycles : 1);
    printf("Ratio load-use/ALU (x10):        %lu\n", ratio10);

    return 0;
}
