"""Cache, replacement, prefetch and memory-controller configuration."""

from __future__ import annotations

from typing import Any

from ._units import _parse_size


class ReplacementPolicy:
    """Namespace for cache replacement policies."""

    class LRU:
        def __repr__(self) -> str:
            return "ReplacementPolicy.LRU()"

    class PLRU:
        def __repr__(self) -> str:
            return "ReplacementPolicy.PLRU()"

    class FIFO:
        def __repr__(self) -> str:
            return "ReplacementPolicy.FIFO()"

    class Random:
        def __repr__(self) -> str:
            return "ReplacementPolicy.Random()"

    class MRU:
        def __repr__(self) -> str:
            return "ReplacementPolicy.MRU()"


class Prefetcher:
    """Namespace for prefetcher configurations."""

    class Off:
        def __repr__(self) -> str:
            return "Prefetcher.Off()"

    class NextLine:
        def __init__(self, degree: int = 1):
            self.degree = degree

        def __repr__(self) -> str:
            return f"Prefetcher.NextLine(degree={self.degree})"

    class Stride:
        def __init__(self, degree: int = 1, table_size: int = 64):
            self.degree = degree
            self.table_size = table_size

        def __repr__(self) -> str:
            return (
                f"Prefetcher.Stride(degree={self.degree}, table_size={self.table_size})"
            )

    class Stream:
        def __init__(self, degree: int = 1):
            self.degree = degree

        def __repr__(self) -> str:
            return f"Prefetcher.Stream(degree={self.degree})"

    class Tagged:
        def __init__(self, degree: int = 1):
            self.degree = degree

        def __repr__(self) -> str:
            return f"Prefetcher.Tagged(degree={self.degree})"


class PageBoundary:
    """Where a load prefetch stream stops at the end of a page."""

    class Stop:
        """Stay inside the page of the load that trained the stream, at that
        page's size: the Cortex-A72 with VA prefetch disabled."""

        def __repr__(self) -> str:
            return "PageBoundary.Stop()"

    class CrossWithTlb:
        """Continue into a page whose translation the data TLB holds, and
        drop the prefetch when it does not: the Cortex-A72's default."""

        def __repr__(self) -> str:
            return "PageBoundary.CrossWithTlb()"


class LoadPrefetcher:
    """Namespace for the load/store unit's load prefetchers
    (``Config(load_prefetcher=...)``)."""

    class Stride:
        """A table indexed by the load's PC, trained on virtual addresses,
        that keeps each confident stream ``l1_lines`` lines ahead in the L1D
        and ``l2_lines`` lines ahead in the L2 (the Cortex-A72 keeps 22)."""

        def __init__(
            self,
            *,
            table_size: int = 64,
            l1_lines: int = 4,
            l2_lines: int = 0,
            page_boundary: PageBoundary.Stop | PageBoundary.CrossWithTlb | None = None,
        ):
            self.table_size = table_size
            self.l1_lines = l1_lines
            self.l2_lines = l2_lines
            self.page_boundary = (
                page_boundary if page_boundary is not None else PageBoundary.Stop()
            )

        def __repr__(self) -> str:
            return (
                f"LoadPrefetcher.Stride(table_size={self.table_size}, "
                f"l1_lines={self.l1_lines}, l2_lines={self.l2_lines}, "
                f"page_boundary={self.page_boundary!r})"
            )


class StorePrefetcher:
    """Namespace for the L1D's store-miss prefetchers
    (``Config(store_prefetcher=...)``)."""

    class Stream:
        """Runs of store misses to adjacent lines in a 4 KiB page, kept
        ``l2_lines`` lines ahead in the L2 with write permission: the
        Cortex-A72's store prefetcher, which fills only the L2."""

        def __init__(self, *, streams: int = 4, l2_lines: int = 8):
            self.streams = streams
            self.l2_lines = l2_lines

        def __repr__(self) -> str:
            return f"StorePrefetcher.Stream(streams={self.streams}, l2_lines={self.l2_lines})"


class MemoryController:
    """Namespace for memory controller configurations."""

    class Simple:
        """Fixed-latency controller: answers ``latency`` core cycles after it
        starts a request, and starts requests serialised on ``bandwidth_gib_s``."""

        def __init__(self, *, latency: int = 120, bandwidth_gib_s: float = 12.8):
            self.latency = latency
            self.bandwidth_gib_s = bandwidth_gib_s

        def __repr__(self) -> str:
            return (
                f"MemoryController.Simple(latency={self.latency}, "
                f"bandwidth_gib_s={self.bandwidth_gib_s})"
            )

    class DRAM:
        def __init__(
            self,
            t_cas: int = 14,
            t_ras: int = 14,
            t_pre: int = 14,
        ):
            self.t_cas = t_cas
            self.t_ras = t_ras
            self.t_pre = t_pre

        def __repr__(self) -> str:
            return (
                f"MemoryController.DRAM(t_cas={self.t_cas}, t_ras={self.t_ras}, "
                f"t_pre={self.t_pre})"
            )

    class DDR5:
        """Command-level DDR5 controller with JEDEC timing.

        Timing comes from ``speed_bin`` (``"4800B"`` or ``"5600B"``) and may
        be overridden per field with ``timing={"t_rcd": 40, ...}`` in DRAM
        command clocks. The controller runs at the DRAM clock; the core clock
        is ``Config(cpu_clock_mhz=...)``.

        Policies: ``scheduler`` is ``"FrFcfs"`` (default) or ``"Fcfs"``;
        ``refresh`` is ``"AllBank"`` (default) or ``"SameBank"``;
        ``address_mapping`` is ``"RoRaBaChCo"`` (default), ``"RoRaBaCoCh"``
        or ``"RoCoRaBaCh"``; ``ecc`` is ``"None"``, ``"SecDed"`` or
        ``"ChipKill"``. ``power_down_idle_ns`` enables rank power-down after
        that many idle nanoseconds; ``patrol_scrub_ns`` enables ECC patrol
        scrubbing at that interval.
        """

        def __init__(
            self,
            speed_bin: str = "4800B",
            channels: int = 2,
            subchannels_per_channel: int = 2,
            ranks_per_channel: int = 2,
            bank_groups_per_rank: int = 8,
            banks_per_group: int = 4,
            row_bits: int = 16,
            column_bits: int = 6,
            read_queue_entries: int = 64,
            write_queue_entries: int = 64,
            write_high_watermark: int = 54,
            write_low_watermark: int = 32,
            min_writes_per_switch: int = 16,
            frontend_latency_ns: int = 10,
            backend_latency_ns: int = 10,
            scheduler: str = "FrFcfs",
            refresh: str = "AllBank",
            address_mapping: str = "RoRaBaChCo",
            power_down_idle_ns: int | None = None,
            ecc: str = "None",
            patrol_scrub_ns: int | None = None,
            timing: dict[str, int] | None = None,
        ):
            self.speed_bin = speed_bin
            self.channels = channels
            self.subchannels_per_channel = subchannels_per_channel
            self.ranks_per_channel = ranks_per_channel
            self.bank_groups_per_rank = bank_groups_per_rank
            self.banks_per_group = banks_per_group
            self.row_bits = row_bits
            self.column_bits = column_bits
            self.read_queue_entries = read_queue_entries
            self.write_queue_entries = write_queue_entries
            self.write_high_watermark = write_high_watermark
            self.write_low_watermark = write_low_watermark
            self.min_writes_per_switch = min_writes_per_switch
            self.frontend_latency_ns = frontend_latency_ns
            self.backend_latency_ns = backend_latency_ns
            self.scheduler = scheduler
            self.refresh = refresh
            self.address_mapping = address_mapping
            self.power_down_idle_ns = power_down_idle_ns
            self.ecc = ecc
            self.patrol_scrub_ns = patrol_scrub_ns
            self.timing = dict(timing) if timing else {}

        def to_dict(self) -> dict[str, Any]:
            return {
                "speed_bin": self.speed_bin,
                "channels": self.channels,
                "subchannels_per_channel": self.subchannels_per_channel,
                "ranks_per_channel": self.ranks_per_channel,
                "bank_groups_per_rank": self.bank_groups_per_rank,
                "banks_per_group": self.banks_per_group,
                "row_bits": self.row_bits,
                "column_bits": self.column_bits,
                "read_queue_entries": self.read_queue_entries,
                "write_queue_entries": self.write_queue_entries,
                "write_high_watermark": self.write_high_watermark,
                "write_low_watermark": self.write_low_watermark,
                "min_writes_per_switch": self.min_writes_per_switch,
                "frontend_latency_ns": self.frontend_latency_ns,
                "backend_latency_ns": self.backend_latency_ns,
                "scheduler": self.scheduler,
                "refresh": self.refresh,
                "address_mapping": self.address_mapping,
                "power_down_idle_ns": self.power_down_idle_ns,
                "ecc": self.ecc,
                "patrol_scrub_ns": self.patrol_scrub_ns,
                "timing": self.timing,
            }

        def __repr__(self) -> str:
            return (
                f"MemoryController.DDR5(speed_bin={self.speed_bin!r}, "
                f"channels={self.channels}, ranks_per_channel={self.ranks_per_channel}, "
                f"scheduler={self.scheduler!r}, refresh={self.refresh!r})"
            )


class Cache:
    """Single cache level configuration."""

    class NINE:
        """No Inclusion, Non-Exclusive (default)."""

        def __repr__(self) -> str:
            return "Cache.NINE()"

    class Inclusive:
        """Inclusive: L2 eviction back-invalidates matching L1 lines."""

        def __repr__(self) -> str:
            return "Cache.Inclusive()"

    class Exclusive:
        """Exclusive: L1 eviction installs line into L2 (swap)."""

        def __repr__(self) -> str:
            return "Cache.Exclusive()"

    def __init__(
        self,
        size: str | int = "4KB",
        line: str | int = "64B",
        ways: int = 1,
        policy: ReplacementPolicy.LRU
        | ReplacementPolicy.PLRU
        | ReplacementPolicy.FIFO
        | ReplacementPolicy.Random
        | ReplacementPolicy.MRU
        | None = None,
        latency: int = 1,
        prefetcher: Prefetcher.Off
        | Prefetcher.NextLine
        | Prefetcher.Stride
        | Prefetcher.Stream
        | Prefetcher.Tagged
        | None = None,
        mshr_count: int = 0,
        write_buffers: int = 0,
        targets_per_mshr: int = 0,
        response_latency: int = 1,
    ):
        self.size_bytes = _parse_size(size)
        self.line_bytes = _parse_size(line)
        self.ways = ways
        self.policy = policy if policy is not None else ReplacementPolicy.LRU()
        self.latency = latency
        self.prefetcher = prefetcher if prefetcher is not None else Prefetcher.Off()
        self.mshr_count = mshr_count
        self.write_buffers = write_buffers
        self.targets_per_mshr = targets_per_mshr
        self.response_latency = response_latency

    def __repr__(self) -> str:
        return (
            f"Cache(size={self.size_bytes}, line={self.line_bytes}, "
            f"ways={self.ways}, policy={self.policy!r}, "
            f"latency={self.latency}, prefetcher={self.prefetcher!r}, "
            f"mshr_count={self.mshr_count}, write_buffers={self.write_buffers}, "
            f"targets_per_mshr={self.targets_per_mshr}, "
            f"response_latency={self.response_latency})"
        )


# Disabled cache dict for levels set to None
_DISABLED_CACHE_DICT: dict[str, Any] = {
    "enabled": False,
    "size_bytes": 4096,
    "line_bytes": 64,
    "ways": 1,
    "policy": "LRU",
    "latency": 1,
    "prefetcher": "None",
    "prefetch_table_size": 0,
    "prefetch_degree": 0,
}


_DISABLED_CACHE_DICT_ZERO: dict[str, Any] = {
    "enabled": False,
    "size_bytes": 0,
    "line_bytes": 0,
    "ways": 0,
    "policy": "LRU",
    "latency": 0,
    "prefetcher": "None",
    "prefetch_table_size": 0,
    "prefetch_degree": 0,
}
