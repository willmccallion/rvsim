#ifndef RVSIM_H
#define RVSIM_H

// Controls the simulator through its sim-control device: stats snapshots
// that bracket a region, stats resets, stops for the host, and exit.

#define RVSIM_SIM_CONTROL_BASE 0x00102000UL

enum rvsim_command {
  RVSIM_RESET_STATS = 1,
  RVSIM_DUMP_STATS = 2,
  RVSIM_EXIT = 3,
  RVSIM_BREAK = 4,
};

static inline void rvsim_command(enum rvsim_command command, unsigned long arg) {
  volatile unsigned long *device = (volatile unsigned long *)RVSIM_SIM_CONTROL_BASE;
  device[1] = arg;
  device[0] = command;
}

// Keeps a snapshot of the stats labelled `label`; the host subtracts two
// snapshots to get the stats of the region between them.
static inline void rvsim_dump_stats(unsigned long label) {
  rvsim_command(RVSIM_DUMP_STATS, label);
}

// Zeroes the stats.
static inline void rvsim_reset_stats(void) { rvsim_command(RVSIM_RESET_STATS, 0); }

// Stops the host's run here, labelled `label`, for it to save, switch
// configuration or measure from this point.
static inline void rvsim_break(unsigned long label) { rvsim_command(RVSIM_BREAK, label); }

// Ends the simulation with exit code `code`.
static inline void rvsim_exit(unsigned long code) { rvsim_command(RVSIM_EXIT, code); }

#endif
