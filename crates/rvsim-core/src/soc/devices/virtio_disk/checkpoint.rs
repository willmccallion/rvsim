//! Saving and restoring the device's state, including the sectors written
//! since the image was loaded.

use super::VirtioBlock;
use super::{SECTOR_SIZE, VirtioBlockState, WrittenSector, from_hex, to_hex};

impl VirtioBlock {
    /// The registers a checkpoint carries.
    #[must_use]
    pub fn state(&self) -> VirtioBlockState {
        VirtioBlockState {
            status: self.status,
            queue_num: self.queue_num,
            queue_ready: self.queue_ready,
            queue_notify: self.queue_notify,
            queue_desc_low: self.queue_desc_low,
            queue_desc_high: self.queue_desc_high,
            queue_avail_low: self.queue_avail_low,
            queue_avail_high: self.queue_avail_high,
            queue_used_low: self.queue_used_low,
            queue_used_high: self.queue_used_high,
            interrupt_status: self.interrupt_status,
            last_avail_idx: self.last_avail_idx,
            device_features_sel: self.device_features_sel,
            driver_features_sel: self.driver_features_sel,
            next_dma_seq: self.next_dma_seq,
            image_digest: self.image_digest,
            written: self
                .written
                .iter()
                .map(|&sector| {
                    let start = (sector * SECTOR_SIZE) as usize;
                    let end = (start + SECTOR_SIZE as usize).min(self.disk_image.len());
                    WrittenSector { sector, data: to_hex(&self.disk_image[start..end]) }
                })
                .collect(),
        }
    }

    /// Restores registers and written sectors from a checkpoint; no
    /// request is in flight afterwards, since a checkpoint is taken
    /// drained.
    ///
    /// # Errors
    ///
    /// Fails when the loaded image is not the one the checkpoint was taken
    /// on, or a written sector does not fit it.
    pub fn set_state(&mut self, state: &VirtioBlockState) -> Result<(), String> {
        self.check_state(state)?;
        for written in &state.written {
            let (start, data) = self.decode_written(written)?;
            self.disk_image[start..start + data.len()].copy_from_slice(&data);
            let _ = self.written.insert(written.sector);
        }
        self.status = state.status;
        self.queue_num = state.queue_num;
        self.queue_ready = state.queue_ready;
        self.queue_notify = state.queue_notify;
        self.queue_desc_low = state.queue_desc_low;
        self.queue_desc_high = state.queue_desc_high;
        self.queue_avail_low = state.queue_avail_low;
        self.queue_avail_high = state.queue_avail_high;
        self.queue_used_low = state.queue_used_low;
        self.queue_used_high = state.queue_used_high;
        self.interrupt_status = state.interrupt_status;
        self.last_avail_idx = state.last_avail_idx;
        self.device_features_sel = state.device_features_sel;
        self.driver_features_sel = state.driver_features_sel;
        self.next_dma_seq = state.next_dma_seq;
        self.job = None;
        Ok(())
    }

    /// Checks that `state` was taken on this disk image and its written
    /// sectors fit it.
    ///
    /// # Errors
    ///
    /// Describes the mismatch.
    pub fn check_state(&self, state: &VirtioBlockState) -> Result<(), String> {
        if state.image_digest != self.image_digest {
            return Err("the disk image differs from the one the checkpoint was taken on".into());
        }
        for written in &state.written {
            let _ = self.decode_written(written)?;
        }
        Ok(())
    }

    /// The byte offset and contents of a checkpointed written sector.
    pub(super) fn decode_written(
        &self,
        written: &WrittenSector,
    ) -> Result<(usize, Vec<u8>), String> {
        let data = from_hex(&written.data).ok_or("a written sector is not hex")?;
        let start = usize::try_from(written.sector * SECTOR_SIZE)
            .map_err(|_| "a written sector lies past the end of the disk")?;
        if data.len() > SECTOR_SIZE as usize || start + data.len() > self.disk_image.len() {
            return Err("a written sector lies past the end of the disk".into());
        }
        Ok((start, data))
    }
}
