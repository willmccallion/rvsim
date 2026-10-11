"""Pipeline backends and their functional units."""

from __future__ import annotations


class Fu:
    """Functional unit pool configuration, for either backend.

    Instantiate ``Fu`` with a list of unit descriptors (the inner classes).
    Any FU type omitted will be absent from the pool, so include every type
    your workload exercises::

        Fu([
            Fu.IntAlu(count=4, latency=1),
            Fu.IntMul(count=1, latency=3),
            Fu.IntDiv(count=1, latency=35),
            Fu.FpAdd(count=2, latency=4),
            Fu.FpMul(count=2, latency=5),
            Fu.FpFma(count=2, latency=5),
            Fu.FpDivSqrt(count=1, latency=21),
            Fu.Branch(count=2, latency=1),
            Fu.Mem(count=2, latency=1),
        ])
    """

    class IntAlu:
        """Integer ALU: add, sub, logic, shift, compare, set-less-than."""

        def __init__(self, count: int = 4, latency: int = 1):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.IntAlu(count={self.count}, latency={self.latency})"

    class IntMul:
        """Integer multiplier: mul, mulh, mulhsu, mulhu."""

        def __init__(self, count: int = 1, latency: int = 3):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.IntMul(count={self.count}, latency={self.latency})"

    class IntDiv:
        """Integer divider: div, divu, rem, remu. Non-pipelined."""

        def __init__(self, count: int = 1, latency: int = 35):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.IntDiv(count={self.count}, latency={self.latency})"

    class FpAdd:
        """FP adder: fadd, fsub, fmin, fmax, fcmp, fcvt."""

        def __init__(self, count: int = 2, latency: int = 4):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.FpAdd(count={self.count}, latency={self.latency})"

    class FpMul:
        """FP multiplier: fmul."""

        def __init__(self, count: int = 2, latency: int = 5):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.FpMul(count={self.count}, latency={self.latency})"

    class FpFma:
        """FP fused multiply-add: fmadd, fmsub, fnmadd, fnmsub."""

        def __init__(self, count: int = 2, latency: int = 5):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.FpFma(count={self.count}, latency={self.latency})"

    class FpDivSqrt:
        """FP divider/sqrt: fdiv, fsqrt. Non-pipelined."""

        def __init__(self, count: int = 1, latency: int = 21):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.FpDivSqrt(count={self.count}, latency={self.latency})"

    class Branch:
        """Branch/jump unit: all conditional branches, jal, jalr."""

        def __init__(self, count: int = 2, latency: int = 1):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.Branch(count={self.count}, latency={self.latency})"

    class Mem:
        """Memory address calculation for loads and stores."""

        def __init__(self, count: int = 2, latency: int = 1):
            self.count = count
            self.latency = latency

        def __repr__(self) -> str:
            return f"Fu.Mem(count={self.count}, latency={self.latency})"

    # ── Vector FU classes (factory-generated) ──────────────────────────────

    @staticmethod
    def _make_vec_fu(name, doc, default_count, default_latency):
        def init(self, count=default_count, latency=default_latency):
            self.count = count
            self.latency = latency

        def repr_(self):
            return f"Fu.{name}(count={self.count}, latency={self.latency})"

        return type(
            name,
            (),
            {
                "__init__": init,
                "__repr__": repr_,
                "__doc__": doc,
                "__module__": __name__,
                "__qualname__": f"Fu.{name}",
            },
        )

    # The out-of-order default core's pool.
    _DEFAULTS: list
    # The in-order backend's default: a dual-issue core's pool.
    _IN_ORDER_DEFAULTS: list

    def __init__(self, units=None):
        self.units = list(units) if units is not None else list(Fu._DEFAULTS)

    def __repr__(self) -> str:
        inner = ", ".join(repr(u) for u in self.units)
        return f"Fu([{inner}])"


# Attach vector FU classes to Fu namespace
for _name, _doc, _count, _lat in [
    ("VecIntAlu", "Vector integer ALU", 1, 1),
    ("VecIntMul", "Vector integer multiplier", 1, 3),
    ("VecIntDiv", "Vector integer divider", 1, 20),
    ("VecFpAlu", "Vector FP ALU", 1, 4),
    ("VecFpFma", "Vector FP FMA", 1, 5),
    ("VecFpDivSqrt", "Vector FP div/sqrt", 1, 20),
    ("VecMem", "Vector memory unit", 1, 1),
    ("VecPermute", "Vector permute unit", 1, 1),
]:
    setattr(Fu, _name, Fu._make_vec_fu(_name, _doc, _count, _lat))


Fu._DEFAULTS = [
    Fu.IntAlu(count=4, latency=1),
    Fu.IntMul(count=1, latency=3),
    Fu.IntDiv(count=1, latency=14),
    Fu.FpAdd(count=2, latency=3),
    Fu.FpMul(count=2, latency=4),
    Fu.FpFma(count=2, latency=4),
    Fu.FpDivSqrt(count=1, latency=13),
    Fu.Branch(count=2, latency=1),
    Fu.Mem(count=3, latency=1),
    Fu.VecIntAlu(count=2, latency=1),
    Fu.VecIntMul(count=1, latency=3),
    Fu.VecIntDiv(count=1, latency=14),
    Fu.VecFpAlu(count=2, latency=3),
    Fu.VecFpFma(count=2, latency=4),
    Fu.VecFpDivSqrt(count=1, latency=13),
    Fu.VecMem(count=2, latency=1),
    Fu.VecPermute(count=1, latency=1),
]

Fu._IN_ORDER_DEFAULTS = [
    Fu.IntAlu(count=2, latency=1),
    Fu.IntMul(count=1, latency=3),
    Fu.IntDiv(count=1, latency=14),
    Fu.FpAdd(count=1, latency=3),
    Fu.FpMul(count=1, latency=4),
    Fu.FpFma(count=1, latency=4),
    Fu.FpDivSqrt(count=1, latency=13),
    Fu.Branch(count=1, latency=1),
    Fu.Mem(count=1, latency=1),
    Fu.VecIntAlu(count=1, latency=1),
    Fu.VecIntMul(count=1, latency=3),
    Fu.VecIntDiv(count=1, latency=14),
    Fu.VecFpAlu(count=1, latency=3),
    Fu.VecFpFma(count=1, latency=4),
    Fu.VecFpDivSqrt(count=1, latency=13),
    Fu.VecMem(count=1, latency=1),
    Fu.VecPermute(count=1, latency=1),
]


class Backend:
    """Namespace for pipeline backend configurations."""

    class InOrder:
        """In-order issue from a FIFO onto ``fu_config``'s unit pool, a
        dual-issue core's by default."""

        def __init__(self, fu_config=None):
            self.fu_config = (
                fu_config if fu_config is not None else Fu(Fu._IN_ORDER_DEFAULTS)
            )

        def __repr__(self) -> str:
            return f"Backend.InOrder(fu_config={self.fu_config!r})"

    class OutOfOrder:
        def __init__(
            self,
            rob_size: int = 320,
            store_buffer_size: int = 72,
            issue_queue_size: int = 128,
            load_queue_size: int = 128,
            load_ports: int = 3,
            store_ports: int = 2,
            prf_gpr_size: int = 224,
            prf_fpr_size: int = 192,
            fu_config=None,
            checkpoint_count: int = 0,
            squash_width: int = 8,
            prf_vpr_size: int = 96,
            vec_chaining: bool = True,
            vec_store_buffer_size: int = 8,
            vec_store_forwarding: str = "byte_mask",
        ):
            self.rob_size = rob_size
            self.store_buffer_size = store_buffer_size
            self.issue_queue_size = issue_queue_size
            self.load_queue_size = load_queue_size
            self.load_ports = load_ports
            self.store_ports = store_ports
            self.prf_gpr_size = prf_gpr_size
            self.prf_fpr_size = prf_fpr_size
            self.fu_config = fu_config if fu_config is not None else Fu()
            self.checkpoint_count = checkpoint_count
            self.squash_width = squash_width
            self.prf_vpr_size = prf_vpr_size
            self.vec_chaining = vec_chaining
            self.vec_store_buffer_size = vec_store_buffer_size
            if vec_store_forwarding not in ("byte_mask", "stall", "off"):
                raise ValueError(
                    f"vec_store_forwarding must be 'byte_mask', 'stall', or 'off'; "
                    f"got {vec_store_forwarding!r}"
                )
            self.vec_store_forwarding = vec_store_forwarding

        def __repr__(self) -> str:
            return (
                f"Backend.OutOfOrder(rob_size={self.rob_size}, "
                f"store_buffer_size={self.store_buffer_size}, "
                f"issue_queue_size={self.issue_queue_size}, "
                f"load_queue_size={self.load_queue_size}, "
                f"load_ports={self.load_ports}, "
                f"store_ports={self.store_ports}, "
                f"vec_store_buffer_size={self.vec_store_buffer_size}, "
                f"vec_store_forwarding={self.vec_store_forwarding!r})"
            )
