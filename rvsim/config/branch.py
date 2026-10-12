"""Branch and memory-dependence predictor configuration."""

from __future__ import annotations

from ._core_defaults import COMPONENTS

_TAGE = COMPONENTS["tage"]
_TAGE_SC_L = COMPONENTS["tage_sc_l"]
_SC = _TAGE_SC_L["sc"]
_ITTAGE = _TAGE_SC_L["ittage"]
_LOOP = _TAGE_SC_L["loop_predictor"]
_BHT = COMPONENTS["bht"]
_PERCEPTRON = COMPONENTS["perceptron"]
_TOURNAMENT = COMPONENTS["tournament"]
_STORE_SET = COMPONENTS["store_set"]

_GHR_MAX_BITS = 1024  # Must match GHR_MAX_WORDS * 64 in branch_predictor.rs


_TAGE_RULES = ("TageBase", "Cbp5")


_TAGE_HISTORY_MODES = ("Direction", "PcBits")


_TAGE_MAX_HISTORY = 8192  # Must match MAX_TAGE_HISTORY in config.rs


_TAGE_HASHINGS = ("TageBase", "TageScL")


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

    class Bht:
        """Rocket's branch history table: ``entries`` counters of
        ``counter_bits`` bits indexed by the fetch PC, with
        ``history_length`` bits of global history hashed to ``history_bits``
        bits placed above the PC bits of the index."""

        def __init__(
            self,
            entries: int = _BHT["entries"],
            counter_bits: int = _BHT["counter_bits"],
            history_length: int = _BHT["history_length"],
            history_bits: int = _BHT["history_bits"],
        ):
            self.entries = entries
            self.counter_bits = counter_bits
            self.history_length = history_length
            self.history_bits = history_bits

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.Bht(entries={self.entries}, "
                f"counter_bits={self.counter_bits}, "
                f"history_length={self.history_length}, "
                f"history_bits={self.history_bits})"
            )

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

        @classmethod
        def from_dict(cls, banking: dict | None) -> BranchPredictor.TageBanking | None:
            """The banking a core TAGE dict holds, or None."""
            return None if banking is None else cls(**banking)

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

    class Tage:
        def __init__(
            self,
            num_banks: int = _TAGE["num_banks"],
            table_size: int = _TAGE["table_size"],
            reset_interval: int = _TAGE["reset_interval"],
            history_lengths: list[int] | None = None,
            tag_widths: list[int] | None = None,
            use_alt_counters: int = _TAGE["use_alt_counters"],
            use_alt_bits: int = _TAGE["use_alt_bits"],
            useful_bits: int = _TAGE["useful_bits"],
            max_allocations: int = _TAGE["max_allocations"],
            allocation: str = _TAGE["allocation"],
            update: str = _TAGE["update"],
            history: str = _TAGE["history"],
            path_history_bits: int = _TAGE["path_history_bits"],
            bimodal_entries: int | None = _TAGE["bimodal_entries"],
            bimodal_hysteresis_share_log: int = _TAGE["bimodal_hysteresis_share_log"],
            hashing: str = _TAGE["hashing"],
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
            self.banking = (
                banking
                if banking is not None
                else BranchPredictor.TageBanking.from_dict(_TAGE["banking"])
            )
            self.table_size = table_size
            self.reset_interval = reset_interval
            self.history_lengths = (
                history_lengths
                if history_lengths is not None
                else list(_TAGE["history_lengths"])
            )
            self.tag_widths = (
                tag_widths if tag_widths is not None else list(_TAGE["tag_widths"])
            )
            _validate_history_lengths(
                self.history_lengths, "TAGE history_lengths", _TAGE_MAX_HISTORY
            )

        def __repr__(self) -> str:
            return (
                f"BranchPredictor.Tage(num_banks={self.num_banks}, "
                f"table_size={self.table_size}, "
                f"reset_interval={self.reset_interval}, "
                f"history_lengths={self.history_lengths}, "
                f"tag_widths={self.tag_widths})"
            )

    class Perceptron:
        def __init__(
            self,
            history_length: int = _PERCEPTRON["history_length"],
            table_bits: int = _PERCEPTRON["table_bits"],
        ):
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
            global_size_bits: int = _TOURNAMENT["global_size_bits"],
            local_hist_bits: int = _TOURNAMENT["local_hist_bits"],
            local_pred_bits: int = _TOURNAMENT["local_pred_bits"],
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

        @classmethod
        def from_dict(cls, gehl: dict) -> BranchPredictor.ScGehl:
            """The GEHL component a core SC dict holds."""
            return cls(gehl["lengths"], gehl["log_entries"], gehl["weight_init"])

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

        @classmethod
        def from_dict(cls, local: dict) -> BranchPredictor.ScLocalGehl:
            """The local GEHL component a core SC dict holds."""
            gehl = local["gehl"]
            return cls(
                local["histories"],
                local["index_shift"],
                gehl["lengths"],
                gehl["log_entries"],
                gehl["weight_init"],
                local["mix_pc"],
            )

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
        ``PcBits`` history with a 27-bit path, ``TageScL`` hashing, and
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
            num_banks: int = _TAGE_SC_L["tage"]["num_banks"],
            table_size: int = _TAGE_SC_L["tage"]["table_size"],
            reset_interval: int = _TAGE_SC_L["tage"]["reset_interval"],
            history_lengths: list[int] | None = None,
            tag_widths: list[int] | None = None,
            use_alt_counters: int = _TAGE_SC_L["tage"]["use_alt_counters"],
            use_alt_bits: int = _TAGE_SC_L["tage"]["use_alt_bits"],
            useful_bits: int = _TAGE_SC_L["tage"]["useful_bits"],
            max_allocations: int = _TAGE_SC_L["tage"]["max_allocations"],
            allocation: str = _TAGE_SC_L["tage"]["allocation"],
            update: str = _TAGE_SC_L["tage"]["update"],
            history: str = _TAGE_SC_L["tage"]["history"],
            path_history_bits: int = _TAGE_SC_L["tage"]["path_history_bits"],
            bimodal_entries: int | None = _TAGE_SC_L["tage"]["bimodal_entries"],
            bimodal_hysteresis_share_log: int = _TAGE_SC_L["tage"][
                "bimodal_hysteresis_share_log"
            ],
            hashing: str = _TAGE_SC_L["tage"]["hashing"],
            banking: BranchPredictor.TageBanking | None = None,
            # Loop predictor parameters
            loop_log_size: int = _LOOP["log_size"],
            loop_log_assoc: int = _LOOP["log_assoc"],
            loop_tag_bits: int = _LOOP["tag_bits"],
            loop_iter_bits: int = _LOOP["iter_bits"],
            loop_confidence_bits: int = _LOOP["confidence_bits"],
            loop_age_bits: int = _LOOP["age_bits"],
            loop_use_counter_bits: int = _LOOP["use_counter_bits"],
            loop_use_direction_bit: bool = _LOOP["use_direction_bit"],
            loop_use_hashing: bool = _LOOP["use_hashing"],
            loop_restrict_allocation: bool = _LOOP["restrict_allocation"],
            loop_initial_iter: int = _LOOP["initial_iter"],
            loop_initial_age: int = _LOOP["initial_age"],
            loop_optional_age_reset: bool = _LOOP["optional_age_reset"],
            loop_long_loop_confidence: bool = _LOOP["long_loop_confidence"],
            loop_optional_age_increment: bool = _LOOP["optional_age_increment"],
            # SC parameters
            sc_log_bias: int = _SC["log_bias"],
            sc_counter_bits: int = _SC["counter_bits"],
            sc_weight_bits: int = _SC["weight_bits"],
            sc_bias_weight_init: int = _SC["bias_weight_init"],
            sc_chooser_bits: int = _SC["chooser_bits"],
            sc_threshold_bits: int = _SC["threshold_bits"],
            sc_initial_threshold: int = _SC["initial_threshold"],
            sc_per_pc_threshold_bits: int = _SC["per_pc_threshold_bits"],
            sc_per_pc_threshold_width: int = _SC["per_pc_threshold_width"],
            sc_initial_per_pc_threshold: int = _SC["initial_per_pc_threshold"],
            sc_threshold_weight_step: int = _SC["threshold_weight_step"],
            sc_halve_short_tables: bool = _SC["halve_short_tables"],
            sc_imli_counter_bits: int = _SC["imli_counter_bits"],
            sc_global: BranchPredictor.ScGehl | None = None,
            sc_backward: BranchPredictor.ScGehl | None = None,
            sc_path: BranchPredictor.ScGehl | None = None,
            sc_local: list[BranchPredictor.ScLocalGehl] | None = None,
            sc_imli: BranchPredictor.ScGehl | None = None,
            sc_imli_history: BranchPredictor.ScGehl | None = None,
            # ITTAGE parameters
            ittage_num_banks: int = _ITTAGE["num_banks"],
            ittage_table_size: int = _ITTAGE["table_size"],
            ittage_history_lengths: list[int] | None = None,
            ittage_tag_widths: list[int] | None = None,
            ittage_reset_interval: int = _ITTAGE["reset_interval"],
        ):
            self.num_banks = num_banks
            self.table_size = table_size
            self.reset_interval = reset_interval
            self.history_lengths = (
                history_lengths
                if history_lengths is not None
                else list(_TAGE_SC_L["tage"]["history_lengths"])
            )
            self.tag_widths = (
                tag_widths
                if tag_widths is not None
                else list(_TAGE_SC_L["tage"]["tag_widths"])
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
                else BranchPredictor.TageBanking.from_dict(
                    _TAGE_SC_L["tage"]["banking"]
                )
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
            self.sc_global = (
                sc_global if sc_global is not None else gehl.from_dict(_SC["global"])
            )
            self.sc_backward = (
                sc_backward
                if sc_backward is not None
                else gehl.from_dict(_SC["backward"])
            )
            self.sc_path = (
                sc_path if sc_path is not None else gehl.from_dict(_SC["path"])
            )
            self.sc_local = (
                list(sc_local)
                if sc_local is not None
                else [local.from_dict(entry) for entry in _SC["local"]]
            )
            self.sc_imli = (
                sc_imli if sc_imli is not None else gehl.from_dict(_SC["imli"])
            )
            self.sc_imli_history = (
                sc_imli_history
                if sc_imli_history is not None
                else gehl.from_dict(_SC["imli_history"])
            )
            self.ittage_num_banks = ittage_num_banks
            self.ittage_table_size = ittage_table_size
            self.ittage_history_lengths = (
                ittage_history_lengths
                if ittage_history_lengths is not None
                else list(_ITTAGE["history_lengths"])
            )
            self.ittage_tag_widths = (
                ittage_tag_widths
                if ittage_tag_widths is not None
                else list(_ITTAGE["tag_widths"])
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
        def __init__(
            self,
            ssit_size: int = _STORE_SET["ssit_size"],
            lfst_size: int = _STORE_SET["lfst_size"],
        ):
            self.ssit_size = ssit_size
            self.lfst_size = lfst_size

        def __repr__(self) -> str:
            return (
                f"MemDepPredictor.StoreSet(ssit_size={self.ssit_size}, "
                f"lfst_size={self.lfst_size})"
            )


def _tage_arguments(tage: dict) -> dict:
    """``BranchPredictor.Tage`` arguments from a core TAGE dict."""
    return {**tage, "banking": BranchPredictor.TageBanking.from_dict(tage["banking"])}


def _branch_predictor_from(pipeline: dict):
    """The branch predictor a core pipeline dict describes."""
    kind = getattr(BranchPredictor, pipeline["branch_predictor"])
    if kind is BranchPredictor.Tage:
        return BranchPredictor.Tage(**_tage_arguments(pipeline["tage"]))
    if kind is BranchPredictor.ScLTage:
        sc = pipeline["sc"]
        gehl = BranchPredictor.ScGehl.from_dict
        components = {"global", "backward", "path", "local", "imli", "imli_history"}
        return BranchPredictor.ScLTage(
            **_tage_arguments(pipeline["tage"]),
            **{
                f"loop_{field}": value
                for field, value in pipeline["loop_predictor"].items()
            },
            **{
                f"sc_{field}": value
                for field, value in sc.items()
                if field not in components
            },
            sc_global=gehl(sc["global"]),
            sc_backward=gehl(sc["backward"]),
            sc_path=gehl(sc["path"]),
            sc_local=[
                BranchPredictor.ScLocalGehl.from_dict(local) for local in sc["local"]
            ],
            sc_imli=gehl(sc["imli"]),
            sc_imli_history=gehl(sc["imli_history"]),
            **{f"ittage_{field}": value for field, value in pipeline["ittage"].items()},
        )
    if kind is BranchPredictor.Bht:
        return BranchPredictor.Bht(**pipeline["bht"])
    if kind is BranchPredictor.Perceptron:
        return BranchPredictor.Perceptron(**pipeline["perceptron"])
    if kind is BranchPredictor.Tournament:
        return BranchPredictor.Tournament(**pipeline["tournament"])
    return kind()


def _mem_dep_predictor_from(pipeline: dict):
    """The memory dependence predictor a core pipeline dict describes."""
    if pipeline["mem_dep_predictor"] == "Blind":
        return MemDepPredictor.Blind()
    store_set = pipeline["store_set"]
    return MemDepPredictor.StoreSet(
        ssit_size=store_set["ssit_size"], lfst_size=store_set["lfst_size"]
    )
