"""Branch and memory-dependence predictor configuration."""

from __future__ import annotations

_GHR_MAX_BITS = 1024  # Must match GHR_MAX_WORDS * 64 in branch_predictor.rs


_TAGE_RULES = ("tage_base", "cbp5")


_TAGE_HISTORY_MODES = ("direction", "pc_bits")


_TAGE_MAX_HISTORY = 8192  # Must match MAX_TAGE_HISTORY in config.rs


_TAGE_HASHINGS = ("tage_base", "tage_sc_l")


# gem5's TAGE_SC_L_TAGE_64KB: 18 geometric lengths from 6 to 3000, each
# shared by a pair of banks, and the banks it enables (noSkip).
_TAGE_SC_L_64KB_HISTORY_LENGTHS = (
    6,
    6,
    9,
    9,
    12,
    12,
    18,
    18,
    26,
    26,
    37,
    37,
    54,
    54,
    78,
    78,
    112,
    112,
    161,
    161,
    232,
    232,
    335,
    335,
    482,
    482,
    695,
    695,
    1002,
    1002,
    1444,
    1444,
    2081,
    2081,
    3000,
    3000,
)


_TAGE_SC_L_64KB_ENABLED = [
    bank
    in (1, 5, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 23, 25, 27, 31, 35)
    for bank in range(36)
]


def _tage_hashing(hashing: str) -> str:
    """Check a TAGE ``hashing`` name."""
    if hashing not in _TAGE_HASHINGS:
        raise ValueError(
            f"TAGE hashing must be one of {_TAGE_HASHINGS}, got {hashing!r}"
        )
    return hashing


def _tage_history_mode(mode: str) -> str:
    """Check a TAGE ``history`` mode name."""
    if mode not in _TAGE_HISTORY_MODES:
        raise ValueError(
            f"TAGE history must be one of {_TAGE_HISTORY_MODES}, got {mode!r}"
        )
    return mode


def _tage_rule(rule: str, name: str) -> str:
    """Check a TAGE ``allocation`` or ``update`` rule name."""
    if rule not in _TAGE_RULES:
        raise ValueError(f"TAGE {name} must be one of {_TAGE_RULES}, got {rule!r}")
    return rule


def _validate_history_lengths(
    lengths: list[int], name: str, capacity: int = _GHR_MAX_BITS
) -> None:
    """Validate that no history length exceeds the history's capacity."""
    max_len = max(lengths) if lengths else 0
    if max_len > capacity:
        raise ValueError(
            f"{name}: maximum history length {max_len} exceeds the "
            f"history capacity of {capacity} bits. "
            f"All history lengths must be <= {capacity}."
        )


class BranchPredictor:
    """Namespace for branch predictor configurations."""

    class Static:
        def __repr__(self) -> str:
            return "BranchPredictor.Static()"

    class GShare:
        def __repr__(self) -> str:
            return "BranchPredictor.GShare()"

    class TageBanking:
        """TAGE-SC-L's banked tables: banks before ``first_long_bank``
        share an array of ``short_factor`` slices of ``table_size``
        entries, the rest one of ``long_factor``; each pair of banks forms
        a 2-way table, and ``enabled`` (one flag per bank) says which exist."""

        def __init__(
            self,
            short_factor: int,
            long_factor: int,
            first_long_bank: int,
            enabled: list[bool],
        ):
            self.short_factor = short_factor
            self.long_factor = long_factor
            self.first_long_bank = first_long_bank
            self.enabled = list(enabled)

        def to_dict(self) -> dict:
            return {
                "short_factor": self.short_factor,
                "long_factor": self.long_factor,
                "first_long_bank": self.first_long_bank,
                "enabled": self.enabled,
            }

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.TageBanking(short_factor={self.short_factor}, "
                f"long_factor={self.long_factor}, "
                f"first_long_bank={self.first_long_bank})"
            )

    class TAGE:
        def __init__(
            self,
            num_banks: int = 8,
            table_size: int = 2048,
            reset_interval: int = 256_000,
            history_lengths: list[int] | None = None,
            tag_widths: list[int] | None = None,
            use_alt_counters: int = 1,
            use_alt_bits: int = 4,
            useful_bits: int = 2,
            max_allocations: int = 1,
            allocation: str = "tage_base",
            update: str = "tage_base",
            history: str = "direction",
            path_history_bits: int = 16,
            bimodal_entries: int | None = None,
            bimodal_hysteresis_share_log: int = 2,
            hashing: str = "tage_base",
            banking: BranchPredictor.TageBanking | None = None,
        ):
            self.num_banks = num_banks
            self.use_alt_counters = use_alt_counters
            self.use_alt_bits = use_alt_bits
            self.useful_bits = useful_bits
            self.max_allocations = max_allocations
            self.allocation = _tage_rule(allocation, "allocation")
            self.update = _tage_rule(update, "update")
            self.history = _tage_history_mode(history)
            self.path_history_bits = path_history_bits
            self.bimodal_entries = bimodal_entries
            self.bimodal_hysteresis_share_log = bimodal_hysteresis_share_log
            self.hashing = _tage_hashing(hashing)
            self.banking = banking
            self.table_size = table_size
            self.reset_interval = reset_interval
            self.history_lengths = (
                history_lengths
                if history_lengths is not None
                else [5, 11, 22, 44, 89, 178, 356, 712]
            )
            self.tag_widths = (
                tag_widths if tag_widths is not None else [8, 8, 9, 9, 10, 10, 11, 11]
            )
            _validate_history_lengths(
                self.history_lengths, "TAGE history_lengths", _TAGE_MAX_HISTORY
            )

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.TAGE(num_banks={self.num_banks}, "
                f"table_size={self.table_size}, "
                f"reset_interval={self.reset_interval}, "
                f"history_lengths={self.history_lengths}, "
                f"tag_widths={self.tag_widths})"
            )

    class Perceptron:
        def __init__(self, history_length: int = 32, table_bits: int = 10):
            self.history_length = history_length
            self.table_bits = table_bits

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.Perceptron(history_length={self.history_length}, "
                f"table_bits={self.table_bits})"
            )

    class Tournament:
        def __init__(
            self,
            global_size_bits: int = 12,
            local_hist_bits: int = 10,
            local_pred_bits: int = 10,
        ):
            self.global_size_bits = global_size_bits
            self.local_hist_bits = local_hist_bits
            self.local_pred_bits = local_pred_bits

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.Tournament(global_size_bits={self.global_size_bits}, "
                f"local_hist_bits={self.local_hist_bits}, "
                f"local_pred_bits={self.local_pred_bits})"
            )

    class ScGehl:
        """One statistical corrector GEHL component: a counter table per
        history length (longest first), each ``2**log_entries`` entries.
        No lengths turns the component off."""

        def __init__(
            self,
            lengths: list[int] | None = None,
            log_entries: int = 0,
            weight_init: int = 0,
        ):
            self.lengths = list(lengths) if lengths is not None else []
            self.log_entries = log_entries
            self.weight_init = weight_init

        def to_dict(self) -> dict:
            return {
                "lengths": self.lengths,
                "log_entries": self.log_entries,
                "weight_init": self.weight_init,
            }

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.ScGehl(lengths={self.lengths}, "
                f"log_entries={self.log_entries}, weight_init={self.weight_init})"
            )

    class ScLocalGehl:
        """A statistical corrector GEHL over per-branch local histories:
        ``histories`` of them (a power of two), a branch's at
        ``(pc ^ (pc >> index_shift)) % histories``; ``mix_pc`` XORs the
        branch's ``pc & 15`` into each update."""

        def __init__(
            self,
            histories: int,
            index_shift: int,
            lengths: list[int],
            log_entries: int,
            weight_init: int = 7,
            mix_pc: bool = False,
        ):
            self.histories = histories
            self.index_shift = index_shift
            self.mix_pc = mix_pc
            self.gehl = BranchPredictor.ScGehl(lengths, log_entries, weight_init)

        def to_dict(self) -> dict:
            return {
                "histories": self.histories,
                "index_shift": self.index_shift,
                "mix_pc": self.mix_pc,
                "gehl": self.gehl.to_dict(),
            }

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.ScLocalGehl(histories={self.histories}, "
                f"index_shift={self.index_shift}, lengths={self.gehl.lengths}, "
                f"log_entries={self.gehl.log_entries}, "
                f"weight_init={self.gehl.weight_init}, mix_pc={self.mix_pc})"
            )

    class ScLTage:
        """SC-L-TAGE + ITTAGE composed predictor.

        Combines TAGE (direction), Loop Predictor, Statistical Corrector,
        and Indirect Target TAGE into a single high-accuracy predictor.

        The TAGE parameters are shared with the standalone TAGE config;
        the defaults add TAGE-SC-L's own TAGE rules (``use_alt_counters=16``,
        CBP-5 ``allocation`` and ``update`` with 1-bit useful counters, two
        allocations, a ``reset_interval`` of 1024 allocation penalties,
        ``pc_bits`` history with a 27-bit path, ``tage_sc_l`` hashing, and
        the 64KB TAGE-SC-L's 36 banked 1024-entry tables over history
        lengths 6 to 3000).
        The loop predictor, SC and ITTAGE have their own sub-configs; the
        loop predictor and SC defaults are Seznec's 64KB TAGE-SC-L (CBP-5).
        The SC's GEHL components are ``BranchPredictor.ScGehl`` and
        ``BranchPredictor.ScLocalGehl`` values; ``None`` takes the default.
        """

        def __init__(
            self,
            # TAGE parameters
            num_banks: int = 36,
            table_size: int = 1024,
            reset_interval: int = 1024,
            history_lengths: list[int] | None = None,
            tag_widths: list[int] | None = None,
            use_alt_counters: int = 16,
            use_alt_bits: int = 5,
            useful_bits: int = 1,
            max_allocations: int = 2,
            allocation: str = "cbp5",
            update: str = "cbp5",
            history: str = "pc_bits",
            path_history_bits: int = 27,
            bimodal_entries: int | None = 8192,
            bimodal_hysteresis_share_log: int = 2,
            hashing: str = "tage_sc_l",
            banking: BranchPredictor.TageBanking | None = None,
            # Loop predictor parameters
            loop_log_size: int = 5,
            loop_log_assoc: int = 2,
            loop_tag_bits: int = 10,
            loop_iter_bits: int = 10,
            loop_confidence_bits: int = 4,
            loop_age_bits: int = 4,
            loop_use_counter_bits: int = 7,
            loop_use_direction_bit: bool = True,
            loop_use_hashing: bool = True,
            loop_restrict_allocation: bool = True,
            loop_initial_iter: int = 0,
            loop_initial_age: int = 7,
            loop_optional_age_reset: bool = False,
            loop_long_loop_confidence: bool = True,
            loop_optional_age_increment: bool = True,
            # SC parameters
            sc_log_bias: int = 8,
            sc_counter_bits: int = 6,
            sc_weight_bits: int = 6,
            sc_bias_weight_init: int = 4,
            sc_chooser_bits: int = 7,
            sc_threshold_bits: int = 12,
            sc_initial_threshold: int = 35,
            sc_per_pc_threshold_bits: int = 6,
            sc_per_pc_threshold_width: int = 8,
            sc_initial_per_pc_threshold: int = 0,
            sc_threshold_weight_step: int = 12,
            sc_halve_short_tables: bool = True,
            sc_imli_counter_bits: int = 8,
            sc_global: BranchPredictor.ScGehl | None = None,
            sc_backward: BranchPredictor.ScGehl | None = None,
            sc_path: BranchPredictor.ScGehl | None = None,
            sc_local: list[BranchPredictor.ScLocalGehl] | None = None,
            sc_imli: BranchPredictor.ScGehl | None = None,
            sc_imli_history: BranchPredictor.ScGehl | None = None,
            # ITTAGE parameters
            ittage_num_banks: int = 8,
            ittage_table_size: int = 256,
            ittage_history_lengths: list[int] | None = None,
            ittage_tag_widths: list[int] | None = None,
            ittage_reset_interval: int = 256_000,
        ):
            self.num_banks = num_banks
            self.table_size = table_size
            self.reset_interval = reset_interval
            self.history_lengths = (
                history_lengths
                if history_lengths is not None
                else list(_TAGE_SC_L_64KB_HISTORY_LENGTHS)
            )
            self.tag_widths = (
                tag_widths if tag_widths is not None else [8] * 12 + [12] * 24
            )
            self.use_alt_counters = use_alt_counters
            self.use_alt_bits = use_alt_bits
            self.useful_bits = useful_bits
            self.max_allocations = max_allocations
            self.allocation = _tage_rule(allocation, "allocation")
            self.update = _tage_rule(update, "update")
            self.history = _tage_history_mode(history)
            self.path_history_bits = path_history_bits
            self.bimodal_entries = bimodal_entries
            self.bimodal_hysteresis_share_log = bimodal_hysteresis_share_log
            self.hashing = _tage_hashing(hashing)
            self.banking = (
                banking
                if banking is not None
                else BranchPredictor.TageBanking(10, 20, 12, _TAGE_SC_L_64KB_ENABLED)
            )
            self.loop_log_size = loop_log_size
            self.loop_log_assoc = loop_log_assoc
            self.loop_tag_bits = loop_tag_bits
            self.loop_iter_bits = loop_iter_bits
            self.loop_confidence_bits = loop_confidence_bits
            self.loop_age_bits = loop_age_bits
            self.loop_use_counter_bits = loop_use_counter_bits
            self.loop_use_direction_bit = loop_use_direction_bit
            self.loop_use_hashing = loop_use_hashing
            self.loop_restrict_allocation = loop_restrict_allocation
            self.loop_initial_iter = loop_initial_iter
            self.loop_initial_age = loop_initial_age
            self.loop_optional_age_reset = loop_optional_age_reset
            self.loop_long_loop_confidence = loop_long_loop_confidence
            self.loop_optional_age_increment = loop_optional_age_increment
            gehl = BranchPredictor.ScGehl
            local = BranchPredictor.ScLocalGehl
            self.sc_log_bias = sc_log_bias
            self.sc_counter_bits = sc_counter_bits
            self.sc_weight_bits = sc_weight_bits
            self.sc_bias_weight_init = sc_bias_weight_init
            self.sc_chooser_bits = sc_chooser_bits
            self.sc_threshold_bits = sc_threshold_bits
            self.sc_initial_threshold = sc_initial_threshold
            self.sc_per_pc_threshold_bits = sc_per_pc_threshold_bits
            self.sc_per_pc_threshold_width = sc_per_pc_threshold_width
            self.sc_initial_per_pc_threshold = sc_initial_per_pc_threshold
            self.sc_threshold_weight_step = sc_threshold_weight_step
            self.sc_halve_short_tables = sc_halve_short_tables
            self.sc_imli_counter_bits = sc_imli_counter_bits
            self.sc_global = sc_global if sc_global is not None else gehl()
            self.sc_backward = (
                sc_backward if sc_backward is not None else gehl([40, 24, 10], 10, 7)
            )
            self.sc_path = sc_path if sc_path is not None else gehl([25, 16, 9], 9, 7)
            self.sc_local = (
                list(sc_local)
                if sc_local is not None
                else [
                    local(256, 2, [11, 6, 3], 10),
                    local(16, 5, [16, 11, 6], 9, mix_pc=True),
                    local(16, 10, [9, 4], 10),
                ]
            )
            self.sc_imli = sc_imli if sc_imli is not None else gehl([8], 8, 7)
            self.sc_imli_history = (
                sc_imli_history if sc_imli_history is not None else gehl([10, 4], 9, 0)
            )
            self.ittage_num_banks = ittage_num_banks
            self.ittage_table_size = ittage_table_size
            self.ittage_history_lengths = (
                ittage_history_lengths
                if ittage_history_lengths is not None
                else [4, 8, 16, 32, 64, 128, 256, 512]
            )
            self.ittage_tag_widths = (
                ittage_tag_widths
                if ittage_tag_widths is not None
                else [9, 9, 10, 10, 11, 11, 12, 12]
            )
            self.ittage_reset_interval = ittage_reset_interval
            _validate_history_lengths(
                self.history_lengths, "ScLTage history_lengths", _TAGE_MAX_HISTORY
            )
            _validate_history_lengths(
                self.ittage_history_lengths, "ScLTage ittage_history_lengths"
            )

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.ScLTage(num_banks={self.num_banks}, "
                f"table_size={self.table_size}, "
                f"sc_local={len(self.sc_local)}, "
                f"ittage_num_banks={self.ittage_num_banks})"
            )


class MemDepPredictor:
    """Namespace for memory dependence predictor configurations."""

    class Blind:
        def __repr__(self) -> str:
            return "MemDepPredictor.Blind()"

    class StoreSet:
        def __init__(self, ssit_size: int = 1024, lfst_size: int = 1024):
            self.ssit_size = ssit_size
            self.lfst_size = lfst_size

        def __repr__(self) -> str:
            return (
                f"MemDepPredictor.StoreSet(ssit_size={self.ssit_size}, "
                f"lfst_size={self.lfst_size})"
            )
