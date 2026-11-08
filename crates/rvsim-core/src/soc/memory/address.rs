//! Physical-address ↔ DRAM-coordinate mapping.
//!
//! [`AddressMapper`] converts a [`PhysAddr`] into a [`DramLocation`] tuple of
//! (channel, subchannel, rank, `bank_group`, bank, row, column) using one of
//! several bit-interleave strategies. All counts must be powers of two;
//! extraction is a set of table-driven bit-slices with no per-request
//! branching.
//!
//! The low [`CACHE_LINE_OFFSET_BITS`] bits of a physical address are the
//! intra-line byte offset and are not part of any DRAM coordinate — every
//! mapping places its lowest-order field at bit [`CACHE_LINE_OFFSET_BITS`],
//! so decomposing an address loses only its offset within the line.

use crate::common::PhysAddr;
use crate::config::AddressMappingKind;
use crate::config::ddr5::CACHE_LINE_OFFSET_BITS;
use crate::sim::components::{BankGroupId, ChannelId, RankId, RowId, SubchannelId};

/// A decomposed physical address in DRAM-topology coordinates.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DramLocation {
    /// Which memory channel.
    pub channel: ChannelId,
    /// Which sub-channel inside the channel.
    pub subchannel: SubchannelId,
    /// Which rank inside the sub-channel.
    pub rank: RankId,
    /// Which bank group inside the rank.
    pub bank_group: BankGroupId,
    /// Bank index inside the bank group.
    pub bank: u8,
    /// Row index inside the bank.
    pub row: RowId,
    /// Column index inside the row (byte granularity).
    pub column: u32,
}

/// Precomputed bit-widths and shifts for a specific mapping configuration.
#[derive(Copy, Clone, Debug, Default)]
pub struct AddressMapper {
    channel: BitField,
    subchannel: BitField,
    rank: BitField,
    bank_group: BitField,
    bank: BitField,
    row: BitField,
    column: BitField,
}

/// The DRAM topology an address maps onto: every count is a power of two.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DramGeometry {
    /// Channels.
    pub channels: u8,
    /// Subchannels per channel.
    pub subchannels: u8,
    /// Ranks per channel.
    pub ranks: u8,
    /// Bank groups per rank.
    pub bank_groups: u8,
    /// Banks per bank group.
    pub banks_per_group: u8,
    /// Row address bits.
    pub row_bits: u8,
    /// Column address bits.
    pub column_bits: u8,
}

/// One coordinate an address carries.
#[derive(Clone, Copy, Debug)]
enum Field {
    Channel,
    Subchannel,
    Rank,
    BankGroup,
    Bank,
    Row,
    Column,
}

/// Where a field sits in an address.
#[derive(Clone, Copy, Debug, Default)]
struct BitField {
    shift: u8,
    bits: u8,
}

impl BitField {
    #[inline]
    const fn mask(self) -> u64 {
        if self.bits == 0 { 0 } else { (1u64 << self.bits) - 1 }
    }

    #[inline]
    const fn extract(self, raw: u64) -> u64 {
        (raw >> self.shift) & self.mask()
    }

    #[cfg(test)]
    #[inline]
    const fn place(self, value: u64) -> u64 {
        (value & self.mask()) << self.shift
    }
}

/// `kind`'s fields from low to high, above the line offset.
const fn layout(kind: AddressMappingKind) -> [Field; 7] {
    use Field::{Bank, BankGroup, Channel, Column, Rank, Row, Subchannel};
    match kind {
        AddressMappingKind::RoRaBaChCo => [Column, Subchannel, Channel, Bank, BankGroup, Rank, Row],
        AddressMappingKind::RoRaBaCoCh => [Channel, Subchannel, Column, Bank, BankGroup, Rank, Row],
        AddressMappingKind::RoCoRaBaCh => [Channel, Subchannel, Bank, BankGroup, Rank, Column, Row],
    }
}

impl DramGeometry {
    /// How many address bits `field` takes.
    const fn bits(self, field: Field) -> u8 {
        match field {
            Field::Channel => self.channels.trailing_zeros() as u8,
            Field::Subchannel => self.subchannels.trailing_zeros() as u8,
            Field::Rank => self.ranks.trailing_zeros() as u8,
            Field::BankGroup => self.bank_groups.trailing_zeros() as u8,
            Field::Bank => self.banks_per_group.trailing_zeros() as u8,
            Field::Row => self.row_bits,
            Field::Column => self.column_bits,
        }
    }
}

impl AddressMapper {
    /// Constructs a mapper for `geometry`, laid out as `kind` says.
    ///
    /// # Panics
    ///
    /// Panics if a count in `geometry` is not a power of two. Treated as a
    /// programmer-error invariant — callers must validate config before
    /// construction.
    #[must_use]
    pub fn new(kind: AddressMappingKind, geometry: DramGeometry) -> Self {
        assert!(geometry.channels.is_power_of_two(), "channels must be a power of two");
        assert!(geometry.subchannels.is_power_of_two(), "subchannels must be a power of two");
        assert!(geometry.ranks.is_power_of_two(), "ranks must be a power of two");
        assert!(geometry.bank_groups.is_power_of_two(), "bank_groups must be a power of two");
        assert!(
            geometry.banks_per_group.is_power_of_two(),
            "banks_per_group must be a power of two"
        );

        let mut mapper = Self::default();
        let mut shift = CACHE_LINE_OFFSET_BITS;
        for field in layout(kind) {
            let bits = geometry.bits(field);
            *mapper.field_mut(field) = BitField { shift, bits };
            shift += bits;
        }
        mapper
    }

    const fn field_mut(&mut self, field: Field) -> &mut BitField {
        match field {
            Field::Channel => &mut self.channel,
            Field::Subchannel => &mut self.subchannel,
            Field::Rank => &mut self.rank,
            Field::BankGroup => &mut self.bank_group,
            Field::Bank => &mut self.bank,
            Field::Row => &mut self.row,
            Field::Column => &mut self.column,
        }
    }

    /// Splits a physical address into DRAM coordinates.
    #[must_use]
    pub const fn decompose(&self, addr: PhysAddr) -> DramLocation {
        let raw = addr.val();
        DramLocation {
            channel: ChannelId::new(self.channel.extract(raw) as u8),
            subchannel: SubchannelId::new(self.subchannel.extract(raw) as u8),
            rank: RankId::new(self.rank.extract(raw) as u8),
            bank_group: BankGroupId::new(self.bank_group.extract(raw) as u8),
            bank: self.bank.extract(raw) as u8,
            row: RowId::new(self.row.extract(raw) as u32),
            column: self.column.extract(raw) as u32,
        }
    }

    #[cfg(test)]
    /// Reassembles a physical address from DRAM coordinates. Inverse of
    /// [`Self::decompose`] modulo field masking.
    #[must_use]
    pub const fn compose(&self, loc: DramLocation) -> PhysAddr {
        PhysAddr::new(
            self.channel.place(loc.channel.val() as u64)
                | self.subchannel.place(loc.subchannel.as_index() as u64)
                | self.rank.place(loc.rank.val() as u64)
                | self.bank_group.place(loc.bank_group.val() as u64)
                | self.bank.place(loc.bank as u64)
                | self.row.place(loc.row.val() as u64)
                | self.column.place(loc.column as u64),
        )
    }
}
