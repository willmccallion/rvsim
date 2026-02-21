"""
Pipeline snapshot with built-in ASCII visualizer.

``cpu.pipeline_snapshot()`` returns a :class:`PipelineSnapshot` whose
``.render()`` / ``.visualize()`` methods display all inter-stage latch
contents as a Gantt-style diagram::

    cpu.tick()
    snap = cpu.pipeline_snapshot()
    snap.visualize()        # print to stdout
    text = snap.render()    # get as string
"""

from ._core import PipelineSnapshot

__all__ = ["PipelineSnapshot"]
