//! `vsetvl` execution shared by the backends.

use crate::exec::vector::vector_config;
use crate::uarch::ctx::StageCtx;
use crate::uarch::pipeline::latches::RenameIssueEntry;
use crate::uarch::pipeline::rob::Rob;

/// Executes a `vsetvl`: records the configuration it establishes on its ROB
/// entry, where younger instructions read it from now and commit writes it
/// to the CSRs. Returns the new `vl`.
pub fn set_vector_config(
    state: &StageCtx<'_>,
    issued: &RenameIssueEntry,
    rs1_value: u64,
    rs2_value: u64,
    rob: &mut Rob,
) -> u64 {
    let current_vl =
        rob.youngest_vec_csr_update().map_or_else(|| state.hart().csrs.vl, |config| config.vl);
    let vlen = state.hart().regs.vpr().vlen();
    let elen = state.config.isa.vector.elen;
    let config = vector_config(&issued.inst, rs1_value, rs2_value, vlen, elen, current_vl);
    rob.set_vec_csr_update(issued.rob_tag, config);
    config.vl
}
