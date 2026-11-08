//! `AddressMapper` unit tests.
//!
//! Round-trips cache-line-aligned physical addresses through decompose/compose
//! for each mapping kind, and checks the interleave properties that
//! distinguish the strategies (channel-line interleave vs. row-locality).

use crate::common::PhysAddr;
use crate::config::AddressMappingKind;
use crate::config::ddr5::CACHE_LINE_OFFSET_BITS;
use crate::soc::memory::address::{AddressMapper, DramGeometry};

/// A standard-ish DDR5 topology used by most tests below:
/// 2 channels, 2 subchannels, 2 ranks, 4 bank groups, 4 banks/group,
/// 15-bit rows, 10-bit columns.
fn make(kind: AddressMappingKind) -> AddressMapper {
    let geometry = DramGeometry {
        channels: 2,
        subchannels: 2,
        ranks: 2,
        bank_groups: 4,
        banks_per_group: 4,
        row_bits: 15,
        column_bits: 10,
    };
    AddressMapper::new(kind, geometry)
}

const LINE_BYTES: u64 = 1 << CACHE_LINE_OFFSET_BITS;

/// Verifies decompose∘compose is identity for a range of line-aligned
/// addresses. Uses an LCG walk over the address space to hit varied bit
/// patterns in every field.
fn assert_roundtrip(kind: AddressMappingKind) {
    let mapper = make(kind);
    let mut raw = 0x1234_5678u64;
    // Mask covers all fields (1+1+1+2+2+15+10 = 32 above line offset) plus the
    // 6-bit line offset. Effective width = 38 bits.
    let addressable_mask = (1u64 << 38) - 1;
    for _ in 0..1024 {
        let aligned = raw & addressable_mask & !(LINE_BYTES - 1);
        let loc = mapper.decompose(PhysAddr::new(aligned));
        let round = mapper.compose(loc);
        assert_eq!(round.val(), aligned, "roundtrip failed at aligned={aligned:#x} kind={kind:?}");
        raw = raw.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    }
}

#[test]
fn roundtrip_rorabachco() {
    assert_roundtrip(AddressMappingKind::RoRaBaChCo);
}

#[test]
fn roundtrip_rorabacoch() {
    assert_roundtrip(AddressMappingKind::RoRaBaCoCh);
}

#[test]
fn roundtrip_rocorabach() {
    assert_roundtrip(AddressMappingKind::RoCoRaBaCh);
}

#[test]
fn stride_of_cache_line_hits_different_channel_under_rorabacoch() {
    // RoRaBaCoCh places channel immediately above the cache-line offset.
    // With 2 channels (1 channel bit at bit 6) and 2 subchannels (1 bit at
    // bit 7), stepping by 64B toggles the channel; by 128B toggles the
    // subchannel; by 256B rolls both back.
    let mapper = make(AddressMappingKind::RoRaBaCoCh);
    let a = mapper.decompose(PhysAddr::new(0));
    let b = mapper.decompose(PhysAddr::new(64));
    let c = mapper.decompose(PhysAddr::new(128));
    let d = mapper.decompose(PhysAddr::new(192));

    assert_eq!(a.channel.val(), 0);
    assert_eq!(b.channel.val(), 1);
    assert_eq!(c.channel.val(), 0);
    assert_eq!(d.channel.val(), 1);
}

#[test]
fn same_row_stays_when_striding_within_row() {
    // RoRaBaChCo (default): from low to high above the line offset, the field
    // order is Column | Subchannel | Channel | Bank | BankGroup | Rank | Row.
    // With column_bits=10 the column field spans 2^10 line-sized entries =
    // 64 KiB of raw address before wrapping into the subchannel bit. Stride
    // by cache lines within that window and confirm the row stays put.
    let mapper = make(AddressMappingKind::RoRaBaChCo);
    let base = mapper.decompose(PhysAddr::new(0));
    let column_window_bytes = 1u64 << 10; // 1024 lines × 64B if scaled; here it's 1024 raw units
    for step in 0..column_window_bytes {
        let addr = step * LINE_BYTES;
        let loc = mapper.decompose(PhysAddr::new(addr));
        assert_eq!(
            loc.row, base.row,
            "row changed within column window at addr={addr:#x}: base={base:?} now={loc:?}"
        );
    }
}

/// The DRAM field that takes the address bit `bit`, if any.
fn field_of_bit(mapper: &AddressMapper, bit: u32) -> &'static str {
    let loc = mapper.decompose(PhysAddr::new(1 << bit));
    let fields = [
        ("channel", u64::from(loc.channel.val())),
        ("subchannel", loc.subchannel.as_index() as u64),
        ("rank", u64::from(loc.rank.val())),
        ("bank_group", u64::from(loc.bank_group.val())),
        ("bank", u64::from(loc.bank)),
        ("row", u64::from(loc.row.val())),
        ("column", u64::from(loc.column)),
    ];
    fields.iter().find(|(_, value)| *value != 0).map_or("offset", |(name, _)| *name)
}

/// Each field's width, low to high, from the bit above the line offset.
fn layout(kind: AddressMappingKind) -> Vec<(&'static str, u32)> {
    let mapper = make(kind);
    let mut runs: Vec<(&'static str, u32)> = Vec::new();
    for bit in u32::from(CACHE_LINE_OFFSET_BITS)..u32::from(CACHE_LINE_OFFSET_BITS) + 32 {
        match runs.last_mut() {
            Some((name, width)) if *name == field_of_bit(&mapper, bit) => *width += 1,
            _ => runs.push((field_of_bit(&mapper, bit), 1)),
        }
    }
    runs
}

#[test]
fn each_mapping_places_its_fields_low_to_high_above_the_line_offset() {
    let common_top = [("bank", 2), ("bank_group", 2), ("rank", 1)];
    assert_eq!(
        layout(AddressMappingKind::RoRaBaChCo),
        [
            [("column", 10), ("subchannel", 1), ("channel", 1)].as_slice(),
            &common_top,
            &[("row", 15)]
        ]
        .concat()
    );
    assert_eq!(
        layout(AddressMappingKind::RoRaBaCoCh),
        [
            [("channel", 1), ("subchannel", 1), ("column", 10)].as_slice(),
            &common_top,
            &[("row", 15)]
        ]
        .concat()
    );
    assert_eq!(
        layout(AddressMappingKind::RoCoRaBaCh),
        [
            [("channel", 1), ("subchannel", 1)].as_slice(),
            &common_top,
            &[("column", 10), ("row", 15)]
        ]
        .concat()
    );
}
