//! Checkpoints of a drained system's architectural state.
//!
//! A checkpoint holds each hart's registers (vector ones too), CSRs, PMP,
//! privilege, PC and load reservation, every device's registers, and RAM.
//! Caches, TLBs and predictors are not kept and start cold after a restore,
//! as gem5's do. A checkpoint restores into any configuration with the same
//! harts, RAM size and VLEN, so a system booted on a fast configuration can
//! continue on a detailed one.
//!
//! The file is an 8-byte little-endian header length, the JSON header, then
//! the RAM image.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::common::{PhysAddr, RegIdx};
use crate::core::Hart;
use crate::core::arch::csr::Csrs;
use crate::core::arch::mode::PrivilegeMode;
use crate::core::units::mmu::pmp::PmpEntry;
use crate::sim::simulator::Simulator;

const MAGIC: &str = "rvsim-checkpoint";
/// The checkpoint format this build writes and restores.
pub const VERSION: u64 = 6;

/// RAM is saved in pages of this many bytes; pages of zeros are skipped.
const PAGE_BYTES: usize = 4096;

/// Why a checkpoint could not be written or restored.
#[derive(Debug, thiserror::Error)]
pub enum CheckpointError {
    /// Reading or writing the file failed.
    #[error("checkpoint I/O: {0}")]
    Io(#[from] std::io::Error),
    /// The header is not valid checkpoint JSON.
    #[error("checkpoint header: {0}")]
    Header(#[from] serde_json::Error),
    /// The file is not an rvsim checkpoint.
    #[error("not an rvsim checkpoint")]
    NotACheckpoint,
    /// The checkpoint was written by an incompatible version.
    #[error("checkpoint version {found} cannot be restored; this build restores version {VERSION}")]
    Version {
        /// The file's version.
        found: u64,
    },
    /// A device cannot take its state, such as a disk whose image differs
    /// from the one the checkpoint was taken on.
    #[error("checkpoint device: {0}")]
    Device(String),
    /// The system being restored into differs in a way the state cannot
    /// carry over.
    #[error("checkpoint has {what} {saved}, this system has {current}")]
    Mismatch {
        /// What differs.
        what: &'static str,
        /// The checkpoint's value.
        saved: u64,
        /// This system's value.
        current: u64,
    },
}

#[derive(Serialize, Deserialize)]
struct Header {
    magic: String,
    version: u64,
    cycle: u64,
    direct_mode: bool,
    trace: bool,
    ram_base: u64,
    ram_size: u64,
    vlen_bits: u64,
    harts: Vec<HartState>,
    devices: serde_json::Value,
}

/// One hart's architectural state.
#[derive(Serialize, Deserialize)]
struct HartState {
    pc: u64,
    privilege: u8,
    wfi_waiting: bool,
    sw_seip: bool,
    instructions_retired: u64,
    gpr: Vec<u64>,
    fpr: Vec<u64>,
    /// All 32 vector registers, register 0 first, when the hart has them.
    vpr: Option<Vec<u8>>,
    csrs: Csrs,
    pmp: Vec<PmpEntry>,
    /// The line the hart holds reserved by an LR.
    reservation: Option<u64>,
}

impl HartState {
    fn of(hart: &Hart, reservation: Option<PhysAddr>) -> Self {
        let registers = 0u8..32;
        Self {
            pc: hart.pc,
            privilege: hart.privilege.to_u8(),
            wfi_waiting: hart.wfi_waiting,
            sw_seip: hart.sw_seip,
            instructions_retired: hart.instructions_retired,
            gpr: registers.clone().map(|i| hart.regs.read(RegIdx::new(i))).collect(),
            fpr: registers.map(|i| hart.regs.read_f(RegIdx::new(i))).collect(),
            vpr: hart.regs.has_vpr().then(|| hart.regs.vpr().bytes().to_vec()),
            csrs: hart.csrs.clone(),
            pmp: hart.pmp.entries().to_vec(),
            reservation: reservation.map(|line| line.val()),
        }
    }

    fn apply(&self, hart: &mut Hart) {
        hart.pc = self.pc;
        hart.privilege = PrivilegeMode::from_u8(self.privilege);
        hart.wfi_waiting = self.wfi_waiting;
        hart.sw_seip = self.sw_seip;
        hart.instructions_retired = self.instructions_retired;
        for (i, &value) in (0u8..).zip(&self.gpr) {
            hart.regs.write(RegIdx::new(i), value);
        }
        for (i, &value) in (0u8..).zip(&self.fpr) {
            hart.regs.write_f(RegIdx::new(i), value);
        }
        if let Some(bytes) = &self.vpr
            && hart.regs.has_vpr()
        {
            hart.regs.vpr_mut().set_bytes(bytes);
        }
        hart.csrs = self.csrs.clone();
        hart.pmp.restore(&self.pmp);
    }
}

/// Writes a bitmap of `ram`'s pages that hold anything but zeros, then
/// those pages in order.
fn write_pages(ram: &[u8], out: &mut impl Write) -> std::io::Result<()> {
    let pages: Vec<&[u8]> = ram.chunks(PAGE_BYTES).collect();
    let mut present = vec![0u8; pages.len().div_ceil(8)];
    for (index, page) in pages.iter().enumerate() {
        if page.iter().any(|&byte| byte != 0) {
            present[index / 8] |= 1 << (index % 8);
        }
    }
    out.write_all(&present)?;
    for (index, page) in pages.iter().enumerate() {
        if present[index / 8] & (1 << (index % 8)) != 0 {
            out.write_all(page)?;
        }
    }
    Ok(())
}

/// Reads what [`write_pages`] wrote into `ram`, zeroing the skipped pages.
fn read_pages(input: &mut impl Read, ram: &mut [u8]) -> std::io::Result<()> {
    let mut present = vec![0u8; ram.len().div_ceil(PAGE_BYTES).div_ceil(8)];
    input.read_exact(&mut present)?;
    for (index, page) in ram.chunks_mut(PAGE_BYTES).enumerate() {
        if present[index / 8] & (1 << (index % 8)) != 0 {
            input.read_exact(page)?;
        } else {
            page.fill(0);
        }
    }
    Ok(())
}

const fn mismatch(what: &'static str, saved: u64, current: u64) -> Result<(), CheckpointError> {
    if saved == current { Ok(()) } else { Err(CheckpointError::Mismatch { what, saved, current }) }
}

impl Simulator {
    /// Drains the system and writes its architectural state to `out`.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] when writing fails.
    pub fn save_checkpoint(&mut self, out: &mut impl Write) -> Result<(), CheckpointError> {
        self.drain();
        let state = &self.state;
        let region = state.bus.ram_region();
        let reservations = state.memory.reservations();
        let header = Header {
            magic: MAGIC.into(),
            version: VERSION,
            cycle: state.cycle,
            direct_mode: state.direct_mode,
            trace: state.trace.armed,
            ram_base: region.map_or(0, |r| r.base()),
            ram_size: region.map_or(0, |r| r.size()),
            vlen_bits: state.config.pipeline.vlen as u64,
            harts: state
                .harts
                .iter()
                .map(|hart| HartState::of(hart, reservations.reserved(hart.hart_id)))
                .collect(),
            devices: state.bus.checkpoint_devices(),
        };
        let header = serde_json::to_vec(&header)?;
        out.write_all(&(header.len() as u64).to_le_bytes())?;
        out.write_all(&header)?;
        if let Some(region) = region {
            // SAFETY: the region is `size()` bytes of RAM the bus owns, and
            // nothing writes it while the drained simulator is borrowed here.
            let ram =
                unsafe { std::slice::from_raw_parts(region.as_ptr(), region.size() as usize) };
            write_pages(ram, out)?;
        }
        out.flush()?;
        Ok(())
    }

    /// Replaces the system's architectural state with the checkpoint read
    /// from `input`, leaving caches, TLBs and the coherence home empty.
    ///
    /// # Errors
    ///
    /// Returns [`CheckpointError`] when the file is not a checkpoint this
    /// build restores, or its harts, RAM or VLEN differ from this system's.
    pub fn restore_checkpoint(&mut self, input: &mut impl Read) -> Result<(), CheckpointError> {
        let mut length = [0u8; 8];
        input.read_exact(&mut length)?;
        let mut header = vec![0u8; u64::from_le_bytes(length) as usize];
        input.read_exact(&mut header)?;
        let header: Header = serde_json::from_slice(&header)?;
        if header.magic != MAGIC {
            return Err(CheckpointError::NotACheckpoint);
        }
        if header.version != VERSION {
            return Err(CheckpointError::Version { found: header.version });
        }
        let region = self.state.bus.ram_region();
        mismatch("harts", header.harts.len() as u64, self.state.harts.len() as u64)?;
        mismatch("RAM bytes", header.ram_size, region.map_or(0, |r| r.size()))?;
        mismatch("RAM base", header.ram_base, region.map_or(0, |r| r.base()))?;
        mismatch("VLEN", header.vlen_bits, self.state.config.pipeline.vlen as u64)?;
        self.state.bus.check_device_states(&header.devices).map_err(CheckpointError::Device)?;

        self.drain();
        if let Some(region) = region {
            // SAFETY: the region is `size()` bytes of RAM the bus owns, and
            // nothing else touches it while the drained simulator is
            // borrowed mutably here.
            let ram =
                unsafe { std::slice::from_raw_parts_mut(region.as_ptr(), region.size() as usize) };
            read_pages(input, ram)?;
        }
        self.apply_header(&header)
    }

    fn apply_header(&mut self, header: &Header) -> Result<(), CheckpointError> {
        let state = &mut self.state;
        state.cycle = header.cycle;
        state.direct_mode = header.direct_mode;
        state.trace.armed = header.trace;
        for (hart, saved) in state.harts.iter_mut().zip(&header.harts) {
            saved.apply(hart);
        }
        for (hart, saved) in state.harts.iter().zip(&header.harts) {
            let reservations = state.shared.memory.reservations_mut();
            match saved.reservation {
                Some(line) => reservations.set(hart.hart_id, PhysAddr::new(line)),
                None => reservations.clear(hart.hart_id),
            }
        }
        state.shared.bus.restore_devices(&header.devices).map_err(CheckpointError::Device)?;
        for core in &mut state.cores {
            let units = &mut core.units;
            units.l1_i_cache.invalidate_all();
            units.l1_d_cache.invalidate_all();
            units.l2_cache.invalidate_all();
            units.mmu.dtlb.flush();
            units.mmu.itlb.flush();
            units.mmu.l2_tlb.flush();
        }
        state.shared.l3_cache.invalidate_all();
        if let Some(coherence) = &mut state.shared.coherence {
            coherence.forget_cached_lines();
        }
        self.state.reset_stats();
        self.sync_arch_regs();
        Ok(())
    }
}
