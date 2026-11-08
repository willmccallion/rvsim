"""Pipeline backends and their functional units."""

from __future__ import annotations

from collections.abc import Iterable
from dataclasses import dataclass
from typing import Any

from ._core_defaults import COMPONENTS, MACHINE

# Fu() and each unit's defaults are the default machine's pool.
_FU = MACHINE["pipeline"]["fu_config"]
_IN_ORDER = COMPONENTS["in_order"]
# Backend.OutOfOrder() is the default machine's out-of-order core.
_OUT_OF_ORDER = MACHINE["pipeline"]


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

    @dataclass
    class IntAlu:
        """Integer ALU: add, sub, logic, shift, compare, set-less-than."""

        count: int = _FU["num_int_alu"]
        latency: int = _FU["int_alu_latency"]

    @dataclass
    class IntMul:
        """Integer multiplier: mul, mulh, mulhsu, mulhu."""

        count: int = _FU["num_int_mul"]
        latency: int = _FU["int_mul_latency"]

    @dataclass
    class IntDiv:
        """Integer divider: div, divu, rem, remu. Non-pipelined."""

        count: int = _FU["num_int_div"]
        latency: int = _FU["int_div_latency"]

    @dataclass
    class FpAdd:
        """FP adder: fadd, fsub, fmin, fmax, fcmp, fcvt."""

        count: int = _FU["num_fp_add"]
        latency: int = _FU["fp_add_latency"]

    @dataclass
    class FpMul:
        """FP multiplier: fmul."""

        count: int = _FU["num_fp_mul"]
        latency: int = _FU["fp_mul_latency"]

    @dataclass
    class FpFma:
        """FP fused multiply-add: fmadd, fmsub, fnmadd, fnmsub."""

        count: int = _FU["num_fp_fma"]
        latency: int = _FU["fp_fma_latency"]

    @dataclass
    class FpDivSqrt:
        """FP divider/sqrt: fdiv, fsqrt. Non-pipelined."""

        count: int = _FU["num_fp_div_sqrt"]
        latency: int = _FU["fp_div_sqrt_latency"]

    @dataclass
    class Branch:
        """Branch/jump unit: all conditional branches, jal, jalr."""

        count: int = _FU["num_branch"]
        latency: int = _FU["branch_latency"]

    @dataclass
    class Mem:
        """Memory address calculation for loads and stores."""

        count: int = _FU["num_mem"]
        latency: int = _FU["mem_latency"]

    @dataclass
    class VecIntAlu:
        """Vector integer ALU."""

        count: int = _FU["num_vec_int_alu"]
        latency: int = _FU["vec_int_alu_latency"]

    @dataclass
    class VecIntMul:
        """Vector integer multiplier."""

        count: int = _FU["num_vec_int_mul"]
        latency: int = _FU["vec_int_mul_latency"]

    @dataclass
    class VecIntDiv:
        """Vector integer divider."""

        count: int = _FU["num_vec_int_div"]
        latency: int = _FU["vec_int_div_latency"]

    @dataclass
    class VecFpAlu:
        """Vector FP ALU."""

        count: int = _FU["num_vec_fp_alu"]
        latency: int = _FU["vec_fp_alu_latency"]

    @dataclass
    class VecFpFma:
        """Vector FP FMA."""

        count: int = _FU["num_vec_fp_fma"]
        latency: int = _FU["vec_fp_fma_latency"]

    @dataclass
    class VecFpDivSqrt:
        """Vector FP div/sqrt."""

        count: int = _FU["num_vec_fp_div_sqrt"]
        latency: int = _FU["vec_fp_div_sqrt_latency"]

    @dataclass
    class VecMem:
        """Vector memory unit."""

        count: int = _FU["num_vec_mem"]
        latency: int = _FU["vec_mem_latency"]

    @dataclass
    class VecPermute:
        """Vector permute unit."""

        count: int = _FU["num_vec_permute"]
        latency: int = _FU["vec_permute_latency"]

    # The default machine's pool.
    _DEFAULTS: list[_AnyFuUnit]

    def __init__(self, units: Iterable[_AnyFuUnit] | None = None) -> None:
        self.units = list(units) if units is not None else list(Fu._DEFAULTS)

    def __repr__(self) -> str:
        inner = ", ".join(repr(u) for u in self.units)
        return f"Fu([{inner}])"


_AnyFuUnit = (
    Fu.IntAlu
    | Fu.IntMul
    | Fu.IntDiv
    | Fu.FpAdd
    | Fu.FpMul
    | Fu.FpFma
    | Fu.FpDivSqrt
    | Fu.Branch
    | Fu.Mem
    | Fu.VecIntAlu
    | Fu.VecIntMul
    | Fu.VecIntDiv
    | Fu.VecFpAlu
    | Fu.VecFpFma
    | Fu.VecFpDivSqrt
    | Fu.VecMem
    | Fu.VecPermute
)

# Each unit type and the stem of its count and latency in the core's pool:
# ``num_<stem>`` and ``<stem>_latency``.
_UNITS = (
    (Fu.IntAlu, "int_alu"),
    (Fu.IntMul, "int_mul"),
    (Fu.IntDiv, "int_div"),
    (Fu.FpAdd, "fp_add"),
    (Fu.FpMul, "fp_mul"),
    (Fu.FpFma, "fp_fma"),
    (Fu.FpDivSqrt, "fp_div_sqrt"),
    (Fu.Branch, "branch"),
    (Fu.Mem, "mem"),
    (Fu.VecIntAlu, "vec_int_alu"),
    (Fu.VecIntMul, "vec_int_mul"),
    (Fu.VecIntDiv, "vec_int_div"),
    (Fu.VecFpAlu, "vec_fp_alu"),
    (Fu.VecFpFma, "vec_fp_fma"),
    (Fu.VecFpDivSqrt, "vec_fp_div_sqrt"),
    (Fu.VecMem, "vec_mem"),
    (Fu.VecPermute, "vec_permute"),
)


def _fu_from(pool: dict[str, int]) -> Fu:
    """The unit list a core pool dict describes: every type it has a unit of."""
    return Fu(
        [
            unit(count=pool[f"num_{stem}"], latency=pool[f"{stem}_latency"])
            for unit, stem in _UNITS
            if pool[f"num_{stem}"] > 0
        ]
    )


Fu._DEFAULTS = _fu_from(_FU).units


class Backend:
    """Namespace for pipeline backend configurations."""

    class InOrder:
        """In-order issue from a FIFO onto ``fu_config``'s unit pool, a
        dual-issue core's when ``None``."""

        def __init__(
            self,
            fu_config: Fu | None = None,
            *,
            rob_size: int = _IN_ORDER["rob_size"],
            store_buffer_size: int = _IN_ORDER["store_buffer_size"],
            load_ports: int = _IN_ORDER["load_ports"],
            store_ports: int = _IN_ORDER["store_ports"],
            vec_store_buffer_size: int = _IN_ORDER["vec_store_buffer_size"],
            vec_store_forwarding: str = _IN_ORDER["vec_store_forwarding"],
        ) -> None:
            self.fu_config = (
                fu_config if fu_config is not None else _fu_from(_IN_ORDER["fu_config"])
            )
            self.rob_size = rob_size
            self.store_buffer_size = store_buffer_size
            self.load_ports = load_ports
            self.store_ports = store_ports
            self.vec_store_buffer_size = vec_store_buffer_size
            self.vec_store_forwarding = vec_store_forwarding

        def __repr__(self) -> str:
            return (
                f"Backend.InOrder(fu_config={self.fu_config!r}, rob_size={self.rob_size}, "
                f"store_buffer_size={self.store_buffer_size}, "
                f"load_ports={self.load_ports}, store_ports={self.store_ports})"
            )

    class OutOfOrder:
        def __init__(
            self,
            rob_size: int = _OUT_OF_ORDER["rob_size"],
            store_buffer_size: int = _OUT_OF_ORDER["store_buffer_size"],
            issue_queue_size: int = _OUT_OF_ORDER["issue_queue_size"],
            load_queue_size: int = _OUT_OF_ORDER["load_queue_size"],
            load_ports: int = _OUT_OF_ORDER["load_ports"],
            store_ports: int = _OUT_OF_ORDER["store_ports"],
            prf_gpr_size: int = _OUT_OF_ORDER["prf_gpr_size"],
            prf_fpr_size: int = _OUT_OF_ORDER["prf_fpr_size"],
            fu_config: Fu | None = None,
            checkpoint_count: int = _OUT_OF_ORDER["checkpoint_count"],
            squash_width: int = _OUT_OF_ORDER["squash_width"],
            prf_vpr_size: int = _OUT_OF_ORDER["prf_vpr_size"],
            vec_chaining: bool = _OUT_OF_ORDER["vec_chaining"],
            vec_store_buffer_size: int = _OUT_OF_ORDER["vec_store_buffer_size"],
            vec_store_forwarding: str = _OUT_OF_ORDER["vec_store_forwarding"],
        ) -> None:
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


_AnyBackend = Backend.InOrder | Backend.OutOfOrder


def _backend_from(pipeline: dict[str, Any]) -> _AnyBackend:
    """The backend a core pipeline dict describes, with its unit pool."""
    units = _fu_from(pipeline["fu_config"])
    if pipeline["backend"] == "InOrder":
        return Backend.InOrder(
            fu_config=units,
            rob_size=pipeline["rob_size"],
            store_buffer_size=pipeline["store_buffer_size"],
            load_ports=pipeline["load_ports"],
            store_ports=pipeline["store_ports"],
            vec_store_buffer_size=pipeline["vec_store_buffer_size"],
            vec_store_forwarding=pipeline["vec_store_forwarding"],
        )
    return Backend.OutOfOrder(
        rob_size=pipeline["rob_size"],
        store_buffer_size=pipeline["store_buffer_size"],
        issue_queue_size=pipeline["issue_queue_size"],
        load_queue_size=pipeline["load_queue_size"],
        load_ports=pipeline["load_ports"],
        store_ports=pipeline["store_ports"],
        prf_gpr_size=pipeline["prf_gpr_size"],
        prf_fpr_size=pipeline["prf_fpr_size"],
        fu_config=units,
        checkpoint_count=pipeline["checkpoint_count"],
        squash_width=pipeline["squash_width"],
        prf_vpr_size=pipeline["prf_vpr_size"],
        vec_chaining=pipeline["vec_chaining"],
        vec_store_buffer_size=pipeline["vec_store_buffer_size"],
        vec_store_forwarding=pipeline["vec_store_forwarding"],
    )
