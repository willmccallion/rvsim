"""Multi-core coherence configuration: the home agent and interconnect."""

from __future__ import annotations


class HomeAgent:
    """Namespace for coherence home-agent policies (who must be snooped)."""

    class Broadcast:
        """Track nothing; snoop every other core on every request."""

        def __repr__(self) -> str:
            return "HomeAgent.Broadcast()"

    class SnoopFilter:
        """Exact sharer tracking in a set-associative filter.

        ``capacity_factor`` sizes the filter as a multiple of the aggregate
        private L2 lines; when a set fills, its least recently used line is
        recalled from every holder.
        """

        def __init__(self, capacity_factor: float = 1.5, ways: int = 8):
            self.capacity_factor = capacity_factor
            self.ways = ways

        def __repr__(self) -> str:
            return f"HomeAgent.SnoopFilter(capacity_factor={self.capacity_factor}, ways={self.ways})"


class Interconnect:
    """Namespace for coherence interconnect topologies.

    Every kind takes the cycles a message spends per hop and the bytes a
    port or link moves per cycle.
    """

    class _Kind:
        kind = ""

        def __init__(self, hop_latency: int = 2, bytes_per_cycle: int = 32):
            self.hop_latency = hop_latency
            self.bytes_per_cycle = bytes_per_cycle

        def __repr__(self) -> str:
            return f"Interconnect.{self.kind}(hop_latency={self.hop_latency}, bytes_per_cycle={self.bytes_per_cycle})"

    class Crossbar(_Kind):
        """Any port to any port in one hop (default)."""

        kind = "Crossbar"

    class Ring(_Kind):
        """Bidirectional ring, routed the shorter way; the home is one stop."""

        kind = "Ring"

    class Mesh(_Kind):
        """Square 2-D mesh with XY routing."""

        kind = "Mesh"

    class Torus(_Kind):
        """Square 2-D torus (mesh with wraparound) with XY routing."""

        kind = "Torus"

    class Hypercube(_Kind):
        """Hypercube with dimension-order routing."""

        kind = "Hypercube"


class Coherence:
    """Coherence fabric between the private caches, used when ``hart_count > 1``."""

    def __init__(
        self,
        home_agent: HomeAgent.Broadcast | HomeAgent.SnoopFilter | None = None,
        interconnect: Interconnect.Crossbar
        | Interconnect.Ring
        | Interconnect.Mesh
        | Interconnect.Torus
        | Interconnect.Hypercube
        | None = None,
        txn_entries: int = 32,
    ):
        self.protocol = "MESI"
        self.home_agent = (
            home_agent if home_agent is not None else HomeAgent.SnoopFilter()
        )
        self.interconnect = (
            interconnect if interconnect is not None else Interconnect.Crossbar()
        )
        self.txn_entries = txn_entries

    def __repr__(self) -> str:
        return (
            f"Coherence(home_agent={self.home_agent!r}, interconnect={self.interconnect!r}, "
            f"txn_entries={self.txn_entries})"
        )
