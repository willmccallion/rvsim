//! `vsetvl` execution shared by the backends.

use crate::core::exec::vector::vector_config;
use crate::core::pipeline::latches::RenameIssueEntry;
use crate::core::pipeline::rob::Rob;
use crate::sim::StageCtx;

/// Executes a `vsetvl`: records the configuration it establishes on its ROB
/// entry, where younger instructions read it from now and commit writes it
/// to the CSRs. Returns the new `vl`.
pub fn set_vector_config(
    state: &StageCtx<'_>,
    id: &RenameIssueEntry,
    rs1_value: u64,
    rs2_value: u64,
    rob: &mut Rob,
) -> u64 {
    let current_vl =
        rob.youngest_vec_csr_update().map_or_else(|| state.hart().csrs.vl, |config| config.vl);
    let vlen = state.hart().regs.vpr().vlen();
    let config = vector_config(&id.inst, rs1_value, rs2_value, vlen, current_vl);
    rob.set_vec_csr_update(id.rob_tag, config);
    config.vl
}
