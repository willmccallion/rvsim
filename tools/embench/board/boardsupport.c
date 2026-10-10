/* Embench-IoT board support for rvsim's SE mode. The triggers do nothing:
 * the simulator's cycle count is the measurement, and the exit code (0
 * when the result verified) is the check. */
#include <support.h>

void initialise_board(void) {}

void __attribute__((noinline)) __attribute__((externally_visible)) start_trigger(void) {}

void __attribute__((noinline)) __attribute__((externally_visible)) stop_trigger(void) {}
