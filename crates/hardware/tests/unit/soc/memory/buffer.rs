//! DRAM Buffer Unit Tests.
//!
//! Verifies allocation, read/write at byte and slice level,
//! indexing, and boundary checks.

use riscv_core::soc::memory::buffer::DramBuffer;

// ══════════════════════════════════════════════════════════
// 1. Allocation and size
// ══════════════════════════════════════════════════════════

#[test]
fn buffer_allocation_size() {
    let buf = DramBuffer::new(4096);
    assert_eq!(buf.len(), 4096);
}

#[test]
fn buffer_initial_zeroed() {
    let buf = DramBuffer::new(256);
    for i in 0..256 {
        assert_eq!(buf.read_u8(i), 0, "Byte {} should be 0", i);
    }
}

// ══════════════════════════════════════════════════════════
// 2. Byte read/write
// ══════════════════════════════════════════════════════════

#[test]
fn buffer_write_read_u8() {
    let buf = DramBuffer::new(256);
    buf.write_u8(0, 0xAB);
    buf.write_u8(255, 0xCD);
    assert_eq!(buf.read_u8(0), 0xAB);
    assert_eq!(buf.read_u8(255), 0xCD);
}

#[test]
fn buffer_write_u8_all_values() {
    let buf = DramBuffer::new(256);
    for i in 0..256 {
        buf.write_u8(i, i as u8);
    }
    for i in 0..256 {
        assert_eq!(buf.read_u8(i), i as u8);
    }
}

// ══════════════════════════════════════════════════════════
// 3. Slice read/write
// ══════════════════════════════════════════════════════════

#[test]
fn buffer_write_slice_read_slice() {
    let buf = DramBuffer::new(256);
    let data = [0xDE, 0xAD, 0xBE, 0xEF];
    buf.write_slice(10, &data);
    let read_back = buf.read_slice(10, 4);
    assert_eq!(read_back, &data);
}

#[test]
fn buffer_write_slice_at_end() {
    let buf = DramBuffer::new(256);
    let data = [0x01, 0x02, 0x03, 0x04];
    buf.write_slice(252, &data);
    assert_eq!(buf.read_u8(252), 0x01);
    assert_eq!(buf.read_u8(255), 0x04);
}

// ══════════════════════════════════════════════════════════
// 4. Index trait
// ══════════════════════════════════════════════════════════

#[test]
fn buffer_index_read() {
    let buf = DramBuffer::new(64);
    buf.write_u8(5, 0x42);
    assert_eq!(buf[5], 0x42);
}

#[test]
fn buffer_index_mut_write() {
    let mut buf = DramBuffer::new(64);
    buf[10] = 0xFF;
    assert_eq!(buf.read_u8(10), 0xFF);
}

// ══════════════════════════════════════════════════════════
// 5. Raw pointer
// ══════════════════════════════════════════════════════════

#[test]
fn buffer_as_ptr_not_null() {
    let buf = DramBuffer::new(64);
    assert!(!buf.as_ptr().is_null());
    assert!(!buf.as_mut_ptr().is_null());
}

// ══════════════════════════════════════════════════════════
// 6. Large allocation
// ══════════════════════════════════════════════════════════

#[test]
fn buffer_large_allocation() {
    let size = 1024 * 1024; // 1 MB
    let buf = DramBuffer::new(size);
    assert_eq!(buf.len(), size);

    // Write at end
    buf.write_u8(size - 1, 0xFF);
    assert_eq!(buf.read_u8(size - 1), 0xFF);
}

// ══════════════════════════════════════════════════════════
// 7. Overwrite
// ══════════════════════════════════════════════════════════

#[test]
fn buffer_overwrite_byte() {
    let buf = DramBuffer::new(64);
    buf.write_u8(0, 0xAA);
    assert_eq!(buf.read_u8(0), 0xAA);
    buf.write_u8(0, 0xBB);
    assert_eq!(buf.read_u8(0), 0xBB);
}

#[test]
fn buffer_overwrite_slice() {
    let buf = DramBuffer::new(64);
    buf.write_slice(0, &[1, 2, 3, 4]);
    buf.write_slice(0, &[5, 6, 7, 8]);
    assert_eq!(buf.read_slice(0, 4), &[5, 6, 7, 8]);
}
