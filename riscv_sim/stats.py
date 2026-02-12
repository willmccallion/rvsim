"""
Simulation statistics container with pattern-based querying.

This module provides StatsObject: a dict-like wrapper for backend statistics (cycles, IPC,
cache hits/misses, branch accuracy, etc.) with a query(pattern) method for filtering by key name or regex.
"""
import re
from typing import Any, Dict, List, Union


class StatsObject(dict):
    """
    Dict-like simulation statistics with .query(pattern) for filtering.

    All stats from the backend are accessible as keys. Typical keys include:
    cycles, instructions_retired, ipc, icache_hits, icache_misses, dcache_hits,
    dcache_misses, l2_hits, l2_misses, l3_hits, l3_misses, stalls_mem, stalls_control,
    stalls_data, branch_predictions, branch_mispredictions, branch_accuracy_pct,
    cycles_user, cycles_kernel, cycles_machine, traps_taken, inst_load, inst_store,
    inst_branch, inst_alu, inst_system, inst_fp_load, inst_fp_store, inst_fp_arith,
    inst_fp_fma, inst_fp_div_sqrt.

    Example:
        result.stats["ipc"]
        result.stats.query("miss")   # cache/branch misses
        result.stats.query("branch")
        result.stats.query("^inst_")
    """
    
    def __init__(self, data: Dict[str, Any]):
        super().__init__(data)
        
    def query(self, pattern: str) -> 'StatsObject':
        """
        Search for statistics matching the given pattern (case-insensitive).
        
        Args:
            pattern: A string or regex pattern to match against statistic names (keys).
            
        Returns:
            A new StatsObject containing only the matching statistics.
        """
        matches = {}
        try:
            regex = re.compile(pattern, re.IGNORECASE)
        except re.error:
            regex = None
            
        for key, value in self.items():
            if regex:
                if regex.search(key):
                    matches[key] = value
            elif pattern.lower() in key.lower():
                matches[key] = value
                
        return StatsObject(matches)

    def __repr__(self) -> str:
        """Pretty-print the stats object with aligned key-value pairs."""
        if not self:
            return "StatsObject({})"
        max_key_len = max(len(k) for k in self.keys())
        lines = []
        for key, value in sorted(self.items()):
            lines.append(f"{key:<{max_key_len}} : {value}")
            
        return "\n".join(lines)
