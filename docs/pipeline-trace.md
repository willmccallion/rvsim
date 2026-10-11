# Pipeline trace

A pipeline trace records where every instruction is in every cycle, for
a viewer to draw: fetched, decoded, renamed, issued, executing, in the
memory stages, written back, committed, or squashed. rvsim writes the
Kanata log that [Konata](https://github.com/shioyadan/Konata) displays,
the viewer gem5 users know from its O3PipeView output.

## Taking a trace

From the command line:

```bash
python -m rvsim software/bin/programs/qsort.elf --pipeline-trace qsort.kanata
```

From Python, around any part of a run:

```python
from rvsim import Config, Simulator

cpu = Simulator(Config(), binary="software/bin/programs/qsort.elf")
cpu.run(limit=10_000)                      # skip the start-up
cpu.open_pipeline_trace("qsort.kanata")   # format="kanata" is the default
cpu.run(limit=50_000)
cpu.close_pipeline_trace()
```

The trace is written out whenever a run stops, so a file is complete
after each `run`. Open it in Konata; each hart is a thread, instructions
are numbered in fetch order, and the detail of an instruction shows its
hart, sequence number and encoding.

## What a row shows

| Cell | Stage |
|---|---|
| `F` | Fetching its line, through fetch1 and fetch2 |
| `Dc` | Decode |
| `Rn` | Renamed and placed in the reorder buffer |
| `Ds` | Dispatched into the issue queue (out-of-order backend) |
| `Is` | Selected to issue (out-of-order backend) |
| `Ex` | Executing |
| `Mem` | In the memory stages, translating and accessing |
| `Wb` | Result written back, waiting to commit |
| `Cm` | Committed |

An instruction that is squashed, by a mispredicted branch, a memory
ordering violation or a trap, ends in red at the stage it had reached.
The in-order backend issues straight from rename, so its rows go from
`Rn` to `Ex`.

A trace of a long run is large: about 200 bytes per instruction. Trace
the region of interest, with a `run(limit=...)` before opening it or a
[pseudo-operation](pseudo-ops.md) marker to stop at.

## Adding a format

The pipeline reports to the `PipelineTrace` trait in
`crates/rvsim-core/src/uarch/pipeline/trace`: `fetched`, `decoded`,
`entered(stage)` and `left(retired or squashed)`, each with the cycle,
hart and sequence number. The Kanata writer is one implementation;
another viewer's format is another implementation, selected by the
`format` argument of `open_pipeline_trace`.
