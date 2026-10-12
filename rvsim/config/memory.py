"""Cache, replacement, prefetch and memory-controller configuration."""

from __future__ import annotations

from typing import Any

from ._core_defaults import COMPONENTS
from ._units import _parse_size

_CACHE = COMPONENTS["cache"]
_MEMORY = COMPONENTS["memory"]
_DDR5 = COMPONENTS["ddr5"]
_LOAD_PREFETCHER = COMPONENTS["load_prefetcher"]
_STORE_PREFETCHER = COMPONENTS["store_prefetcher"]


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


# The core's names for the replacement policies.
_POLICIES = {
    "LRU": ReplacementPolicy.LRU,
    "PLRU": ReplacementPolicy.PLRU,
    "FIFO": ReplacementPolicy.FIFO,
    "RANDOM": ReplacementPolicy.Random,
    "MRU": ReplacementPolicy.MRU,
}


class Prefetcher:
    """Namespace for prefetcher configurations."""

    class Off:
        def __repr__(self) -> str:
            return "Prefetcher.Off()"

    class NextLine:
        def __init__(self, degree: int = _CACHE["prefetch_degree"]):
            self.degree = degree

        def __repr__(self) -> str:
            return f"Prefetcher.NextLine(degree={self.degree})"

    class Stride:
        def __init__(
            self,
            degree: int = _CACHE["prefetch_degree"],
            table_size: int = _CACHE["prefetch_table_size"],
        ):
            self.degree = degree
            self.table_size = table_size

        def __repr__(self) -> str:
            return (
                f"Prefetcher.Stride(degree={self.degree}, table_size={self.table_size})"
            )

    class Stream:
        def __init__(self, degree: int = _CACHE["prefetch_degree"]):
            self.degree = degree

        def __repr__(self) -> str:
            return f"Prefetcher.Stream(degree={self.degree})"

    class Tagged:
        def __init__(self, degree: int = _CACHE["prefetch_degree"]):
            self.degree = degree

        def __repr__(self) -> str:
            return f"Prefetcher.Tagged(degree={self.degree})"


# The core's names for the cache-side prefetchers.
_PREFETCHERS = {
    "None": Prefetcher.Off,
    "NextLine": Prefetcher.NextLine,
    "Stride": Prefetcher.Stride,
    "Stream": Prefetcher.Stream,
    "Tagged": Prefetcher.Tagged,
}


def _prefetcher_from(cache: dict[str, Any]):
    """The cache-side prefetcher a core cache-level dict names."""
    kind = _PREFETCHERS[cache["prefetcher"]]
    if kind is Prefetcher.Off:
        return Prefetcher.Off()
    if kind is Prefetcher.Stride:
        return Prefetcher.Stride(
            degree=cache["prefetch_degree"], table_size=cache["prefetch_table_size"]
        )
    return kind(degree=cache["prefetch_degree"])


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


# The core's names for the page boundary rules.
_PAGE_BOUNDARIES = {
    "Stop": PageBoundary.Stop,
    "CrossWithTlb": PageBoundary.CrossWithTlb,
}


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
            table_size: int = _LOAD_PREFETCHER["table_size"],
            l1_lines: int = _LOAD_PREFETCHER["l1_lines"],
            l2_lines: int = _LOAD_PREFETCHER["l2_lines"],
            page_boundary: PageBoundary.Stop | PageBoundary.CrossWithTlb | None = None,
        ):
            self.table_size = table_size
            self.l1_lines = l1_lines
            self.l2_lines = l2_lines
            self.page_boundary = (
                page_boundary
                if page_boundary is not None
                else _PAGE_BOUNDARIES[_LOAD_PREFETCHER["page_boundary"]]()
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

        def __init__(
            self,
            *,
            streams: int = _STORE_PREFETCHER["streams"],
            l2_lines: int = _STORE_PREFETCHER["l2_lines"],
        ):
            self.streams = streams
            self.l2_lines = l2_lines

        def __repr__(self) -> str:
            return f"StorePrefetcher.Stream(streams={self.streams}, l2_lines={self.l2_lines})"


class MemoryController:
    """Namespace for memory controller configurations."""

    class Simple:
        """Fixed-latency controller: answers ``latency`` core cycles after it
        starts a request, and starts requests serialised on ``bandwidth_gib_s``."""

        def __init__(
            self,
            *,
            latency: int = _MEMORY["simple_latency"],
            bandwidth_gib_s: float = _MEMORY["simple_bandwidth_gib_s"],
        ):
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
            t_cas: int = _MEMORY["t_cas"],
            t_ras: int = _MEMORY["t_ras"],
            t_pre: int = _MEMORY["t_pre"],
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
            speed_bin: str = _DDR5["speed_bin"],
            channels: int = _DDR5["channels"],
            subchannels_per_channel: int = _DDR5["subchannels_per_channel"],
            ranks_per_channel: int = _DDR5["ranks_per_channel"],
            bank_groups_per_rank: int = _DDR5["bank_groups_per_rank"],
            banks_per_group: int = _DDR5["banks_per_group"],
            row_bits: int = _DDR5["row_bits"],
            column_bits: int = _DDR5["column_bits"],
            read_queue_entries: int = _DDR5["read_queue_entries"],
            write_queue_entries: int = _DDR5["write_queue_entries"],
            write_high_watermark: int = _DDR5["write_high_watermark"],
            write_low_watermark: int = _DDR5["write_low_watermark"],
            min_writes_per_switch: int = _DDR5["min_writes_per_switch"],
            frontend_latency_ns: int = _DDR5["frontend_latency_ns"],
            backend_latency_ns: int = _DDR5["backend_latency_ns"],
            scheduler: str = _DDR5["scheduler"],
            refresh: str = _DDR5["refresh"],
            address_mapping: str = _DDR5["address_mapping"],
            power_down_idle_ns: int | None = _DDR5["power_down_idle_ns"],
            ecc: str = _DDR5["ecc"],
            patrol_scrub_ns: int | None = _DDR5["patrol_scrub_ns"],
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


# The core's names for the memory controllers.
_CONTROLLERS = {
    "Simple": MemoryController.Simple,
    "Dram": MemoryController.DRAM,
    "Ddr5": MemoryController.DDR5,
}


def _resource_count(name: str, count: int | None) -> int | None:
    """A cache resource count: None for the simulator default, else at least 1."""
    if count is not None and count < 1:
        raise ValueError(
            f"{name} must be at least 1, or None for the default; got {count}"
        )
    return count


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
        size: str | int = _CACHE["size_bytes"],
        line: str | int = _CACHE["line_bytes"],
        ways: int = _CACHE["ways"],
        policy: ReplacementPolicy.LRU
        | ReplacementPolicy.PLRU
        | ReplacementPolicy.FIFO
        | ReplacementPolicy.Random
        | ReplacementPolicy.MRU
        | None = None,
        latency: int = _CACHE["latency"],
        prefetcher: Prefetcher.Off
        | Prefetcher.NextLine
        | Prefetcher.Stride
        | Prefetcher.Stream
        | Prefetcher.Tagged
        | None = None,
        mshr_count: int | None = None,
        write_buffers: int | None = None,
        targets_per_mshr: int | None = None,
        response_latency: int = _CACHE["response_latency"],
    ):
        self.size_bytes = _parse_size(size)
        self.line_bytes = _parse_size(line)
        self.ways = ways
        self.policy = policy if policy is not None else _POLICIES[_CACHE["policy"]]()
        self.latency = latency
        self.prefetcher = (
            prefetcher if prefetcher is not None else _prefetcher_from(_CACHE)
        )
        self.mshr_count = _resource_count("mshr_count", mshr_count)
        self.write_buffers = _resource_count("write_buffers", write_buffers)
        self.targets_per_mshr = _resource_count("targets_per_mshr", targets_per_mshr)
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


# The core's names for the inclusion policies.
_INCLUSION_POLICIES = {
    "Nine": Cache.NINE,
    "Inclusive": Cache.Inclusive,
    "Exclusive": Cache.Exclusive,
}


def _unless_default(level: dict[str, Any], field: str) -> int | None:
    """A cache resource count from a core level dict: None when it is the
    core's default, as a ``Cache`` built without it holds."""
    return None if level[field] == _CACHE[field] else level[field]


def _cache_from(level: dict[str, Any]) -> Cache | None:
    """The cache level a core cache dict describes, or None when it is off."""
    if not level["enabled"]:
        return None
    return Cache(
        size=level["size_bytes"],
        line=level["line_bytes"],
        ways=level["ways"],
        policy=_POLICIES[level["policy"]](),
        latency=level["latency"],
        prefetcher=_prefetcher_from(level),
        mshr_count=_unless_default(level, "mshr_count"),
        write_buffers=_unless_default(level, "write_buffers"),
        targets_per_mshr=_unless_default(level, "targets_per_mshr"),
        response_latency=level["response_latency"],
    )


def _load_prefetcher_from(prefetcher: dict[str, Any]) -> LoadPrefetcher.Stride | None:
    """The load prefetcher a core cache dict names, or None."""
    if prefetcher["kind"] == "None":
        return None
    return LoadPrefetcher.Stride(
        table_size=prefetcher["table_size"],
        l1_lines=prefetcher["l1_lines"],
        l2_lines=prefetcher["l2_lines"],
        page_boundary=_PAGE_BOUNDARIES[prefetcher["page_boundary"]](),
    )


def _store_prefetcher_from(prefetcher: dict[str, Any]) -> StorePrefetcher.Stream | None:
    """The store prefetcher a core cache dict names, or None."""
    if prefetcher["kind"] == "None":
        return None
    return StorePrefetcher.Stream(
        streams=prefetcher["streams"], l2_lines=prefetcher["l2_lines"]
    )


def _controller_from(memory: dict[str, Any]):
    """The memory controller a core memory dict names."""
    kind = _CONTROLLERS[memory["controller"]]
    if kind is MemoryController.Simple:
        return MemoryController.Simple(
            latency=memory["simple_latency"],
            bandwidth_gib_s=memory["simple_bandwidth_gib_s"],
        )
    if kind is MemoryController.DRAM:
        return MemoryController.DRAM(
            t_cas=memory["t_cas"], t_ras=memory["t_ras"], t_pre=memory["t_pre"]
        )
    return MemoryController.DDR5(**memory["ddr5"])
