use {
    std::{
        borrow::Cow,
        iter::{
            self,
            FusedIterator,
        },
        ops::{
            Index,
            Range,
        },
    },
    arrayref::array_ref,
    async_compression::tokio::write::ZlibEncoder,
    itermore::IterArrayWindows as _,
    itertools::{
        Either,
        Itertools as _,
    },
    rand::{
        rng,
        prelude::*,
    },
    tokio::io::{
        self,
        AsyncWrite,
        AsyncWriteExt as _,
    },
};

const DMADATA_START: u32 = 0x7430;
const XOR_RANGE: Range<usize> = 0x00b8_ad30..0x00f0_29a0;
const BLOCK_HEADER_SIZE: usize = 7;

#[derive(Debug, thiserror::Error)]
#[error("DMA entry for DMA table not found. Attempted to find DMA entry starting at {}.", DMADATA_START)]
pub struct DmaTableError;

pub struct Patch<'a> {
    base_rom: &'a [u8],
    /// A list of segments of raw data that should be changed by the patch, identified by their starting addresses and changed data.
    ///
    /// # Invariant
    ///
    /// This vector is sorted at all times, and there is a gap (of at least 1 byte) between adjacent segments.
    changed_segments: Vec<(usize, Cow<'a, [u8]>)>,
}

impl<'a> Patch<'a> {
    fn slice(&self, Range { start, end }: Range<usize>) -> Cow<'_, [u8]> {
        let (mut buf, start_idx) = match self.changed_segments.binary_search_by_key(&start, |&(addr, _)| addr) {
            Ok(found_idx) => {
                let (_, segment) = &self.changed_segments[found_idx];
                if segment.len() >= end - start {
                    return Cow::Borrowed(&segment[..end - start])
                } else {
                    let mut buf = Vec::with_capacity(end - start);
                    buf.extend_from_slice(segment);
                    (buf, found_idx + 1)
                }
            }
            Err(insert_idx) => (Vec::with_capacity(end - start), insert_idx),
        };
        for (start_addr, segment) in &self.changed_segments[start_idx..] {
            if *start_addr >= end {
                buf.extend_from_slice(&self.base_rom[start + buf.len()..end]);
                break
            } else {
                buf.extend_from_slice(&self.base_rom[start + buf.len()..*start_addr]);
                if start_addr + segment.len() >= end {
                    buf.extend_from_slice(&segment[..end - start - buf.len()]);
                    break
                } else {
                    buf.extend_from_slice(segment);
                }
            }
        }
        if buf.len() < end - start {
            buf.extend_from_slice(&self.base_rom[start + buf.len()..end]);
        }
        Cow::Owned(buf)
    }

    async fn write_zpf_xor_block(&self, xor_address: &mut usize, block: Range<usize>, writer: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
        /// get the next XOR key. Uses some location in the source rom.
        /// This will skip of 0s, since if we hit a block of 0s, the
        /// patch data will be raw.
        fn key_next(patch: &Patch<'_>, key_address: &mut usize) -> u8 {
            loop {
                *key_address += 1;
                if *key_address >= XOR_RANGE.end {
                    *key_address = XOR_RANGE.start;
                }
                let key = patch.base_rom[*key_address];
                if key != 0 { break key }
            }
        }

        async fn write_block_section(start: usize, key_skip: u8, in_data: &[u8], is_continue: bool, writer: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
            if !is_continue {
                writer.write_u32(start.try_into().expect("address out of range")).await?;
            } else {
                writer.write_u8(0xff).await?;
                writer.write_u8(key_skip).await?;
            }
            writer.write_u16(in_data.len().try_into().expect("block section too long")).await?;
            writer.write_all(in_data).await?;
            Ok(())
        }

        let mut new_data = Vec::with_capacity(block.len());
        let mut key_offset = 0;
        let mut continue_block = false;
        for address in block.clone() {
            let byte = self[address];
            if byte == 0 {
                // Leave 0s as 0s. Do not XOR
                new_data.push(0);
            } else {
                let mut key = key_next(self, xor_address);
                // if the XOR would result in 0, change the key.
                // This requires breaking up the block.
                if byte == key {
                    write_block_section(block.start, key_offset, &new_data, continue_block, writer).await?;
                    new_data = Vec::with_capacity(block.end - address);
                    key_offset = 0;
                    continue_block = true;
                    // search for next safe XOR key
                    while byte == key {
                        key_offset += 1;
                        key = key_next(self, xor_address);
                        // if we aren't able to find one quickly, we may need to break again
                        if key_offset == 0xff {
                            write_block_section(block.start, key_offset, &new_data, continue_block, writer).await?;
                            new_data = Vec::with_capacity(block.end - address);
                            key_offset = 0;
                            continue_block = true;
                        }
                    }
                }
                // XOR the key with the byte
                new_data.push(byte ^ key);
                // Break the block if it's too long
                if new_data.len() == 0xffff {
                    write_block_section(block.start, key_offset, &new_data, continue_block, writer).await?;
                    new_data = Vec::with_capacity(block.end - address);
                    key_offset = 0;
                    continue_block = true;
                }
            }
        }
        // Save the block
        write_block_section(block.start, key_offset, &new_data, continue_block, writer).await?;
        Ok(())
    }

    pub async fn write_zpf(&self, writer: impl AsyncWrite + Unpin) -> io::Result<()> {
        let mut zpf_buf = ZlibEncoder::new(writer);
        // header
        zpf_buf.write_all(b"ZPFv1").await?;
        zpf_buf.write_u32(DMADATA_START).await?;
        zpf_buf.write_u32(XOR_RANGE.start.try_into().expect("address out of range")).await?;
        zpf_buf.write_u32(XOR_RANGE.end.try_into().expect("address out of range")).await?;
        let mut xor_address = rng().random_range(XOR_RANGE);
        zpf_buf.write_u32(xor_address.try_into().expect("address out of range")).await?;
        // DMA updates
        // _calculate_dma_entries
        let mut i = 0;
        let dma_index = loop {
            if i > 2000 { return Err(io::Error::new(io::ErrorKind::InvalidData, DmaTableError)) }
            if u32::from_be_bytes(self.slice((DMADATA_START + i * 0x10).try_into().expect("address out of range")..(DMADATA_START + i * 0x10 + 0x04).try_into().expect("address out of range")).as_ref().try_into().expect("slice length should be 4")) == DMADATA_START { break i }
            i += 1;
        };
        let dma_end = u32::from_be_bytes(self.slice((DMADATA_START + dma_index * 0x10 + 0x04).try_into().expect("address out of range")..(DMADATA_START + dma_index * 0x10 + 0x08).try_into().expect("address out of range")).as_ref().try_into().expect("slice length should be 4"));
        let num_dma_entries = (dma_end - DMADATA_START) >> 4;
        // scan_dmadata_update
        let mut changed_dma = Vec::default();
        for dma_entry_idx in 0..num_dma_entries {
            let dma_start = u32::from_be_bytes(self.slice((DMADATA_START + dma_entry_idx * 0x10).try_into().expect("address out of range")..(DMADATA_START + i * 0x10 + 0x04).try_into().expect("address out of range")).as_ref().try_into().expect("slice length should be 4"));
            let dma_end = u32::from_be_bytes(self.slice((DMADATA_START + dma_entry_idx * 0x10 + 0x04).try_into().expect("address out of range")..(DMADATA_START + i * 0x10 + 0x08).try_into().expect("address out of range")).as_ref().try_into().expect("slice length should be 4"));
            let old_dma_start = u32::from_be_bytes(*array_ref![self.base_rom, (DMADATA_START + dma_entry_idx * 0x10).try_into().expect("address out of range"), 4]);
            let old_dma_end = u32::from_be_bytes(*array_ref![self.base_rom, (DMADATA_START + dma_entry_idx * 0x10 + 0x04).try_into().expect("address out of range"), 4]);
            if dma_start == 0 && dma_end == 0 && old_dma_start == 0 && old_dma_end == 0 {
                break
            }
            // If the entries do not match, the flag the changed entry
            if !(dma_start == old_dma_start && dma_end == old_dma_end) {
                let from_file = if dma_entry_idx < 1496 { old_dma_start.try_into().expect("address out of range") } else { -1 };
                changed_dma.push((dma_entry_idx, from_file, dma_start, dma_end - dma_start));
            }
        }
        let mut relocations = Vec::with_capacity(changed_dma.len());
        for (dma_index, from_file, start, size) in changed_dma {
            zpf_buf.write_u16(dma_index.try_into().expect("DMA entry index out of range")).await?;
            zpf_buf.write_i32(from_file).await?;
            zpf_buf.write_u32(start).await?;
            let [0, a, b, c] = size.to_be_bytes() else { panic!("DMA update size does not fit into 3 bytes") };
            zpf_buf.write_all(&[a, b, c]).await?;
            // Simulate moving the files to know which addresses have changed
            if let Ok(old_dma_start) = u32::try_from(from_file) {
                let dma_entry_idx = (0..num_dma_entries).find(|dma_entry_idx| u32::from_be_bytes(*array_ref![self.base_rom, (DMADATA_START + dma_entry_idx * 0x10).try_into().expect("address out of range"), 4]) == old_dma_start).expect("changed DMA record not found");
                let old_dma_end = u32::from_be_bytes(*array_ref![self.base_rom, (DMADATA_START + dma_entry_idx * 0x10 + 0x04).try_into().expect("address out of range"), 4]);
                let copy_size = size.min(old_dma_end - old_dma_start);
                let Err(idx) = relocations.binary_search_by_key(&start, |(Range { start, .. }, _)| *start) else { panic!("duplicate relocation") };
                relocations.insert(idx, (start..start + copy_size, Some(old_dma_start)));
                if copy_size < size {
                    relocations.insert(idx + 1, (start + copy_size..start + size, None));
                }
            } else {
                // this is a new file, so we just fill with null data
                let Err(idx) = relocations.binary_search_by_key(&start, |(Range { start, .. }, _)| *start) else { panic!("duplicate relocation") };
                relocations.insert(idx, (start..start + size, None));
            }
        }
        zpf_buf.write_u16(0xffff).await?;
        // XOR data
        let base = self.base_rom[..relocations.first().map(|(from, _)| from.start.try_into().expect("address out of range")).unwrap_or_else(|| self.base_rom.len())].iter().copied()
            .chain(
                relocations.iter().map(|(from, to)| Either::Left(if let Some(to) = *to { Either::Left(self.base_rom[to.try_into().expect("address out of range")..usize::try_from(to).expect("address out of range") + from.len()].iter().copied()) } else { Either::Right(iter::repeat_n(0, from.len())) }))
                .interleave(relocations.iter().array_windows().map(|[(from1, _), (from2, _)]| Either::Right(self.base_rom[from1.end.try_into().expect("address out of range")..from2.start.try_into().expect("address out of range")].iter().copied())))
                .flatten()
            )
            .chain(relocations.last().into_iter().flat_map(|(from, _)| self.base_rom[from.end.try_into().expect("address out of range")..].iter().copied()));
        let patched = self.into_iter();
        let mut block = None;
        for (addr, (base, patched)) in base.zip_eq(patched).enumerate() {
            if (DMADATA_START..dma_end).contains(&addr.try_into().expect("address out of range")) || patched == base { continue } //TODO force_patch support
            // Starting a new block to skip unchanged bytes only actually saves space if we skip more than the size of the block header.
            match block {
                None => block = Some(addr..addr + 1),
                Some(old_block) if addr > old_block.end + BLOCK_HEADER_SIZE => {
                    self.write_zpf_xor_block(&mut xor_address, old_block, &mut zpf_buf).await?;
                    block = Some(addr..addr + 1);
                }
                Some(Range { ref mut end, .. }) => *end = addr + 1,
            }
        }
        if let Some(block) = block {
            self.write_zpf_xor_block(&mut xor_address, block, &mut zpf_buf).await?;
        }
        zpf_buf.shutdown().await?; // write zlib trailer
        zpf_buf.into_inner().flush().await?; // make sure data is actually written to writer
        Ok(())
    }
}

impl<'a> Index<usize> for Patch<'a> {
    type Output = u8;

    fn index(&self, address: usize) -> &u8 {
        match self.changed_segments.binary_search_by_key(&address, |&(addr, _)| addr) {
            Ok(found_idx) => &self.changed_segments[found_idx].1[0],
            Err(insert_idx) => if let Some(prev_idx) = insert_idx.checked_sub(1) {
                let (start_addr, segment) = &self.changed_segments[prev_idx];
                if address < start_addr + segment.len() {
                    &segment[address - start_addr]
                } else {
                    &self.base_rom[address]
                }
            } else {
                &self.base_rom[address]
            },
        }
    }
}

impl<'a, 'b> IntoIterator for &'b Patch<'a> {
    type IntoIter = PatchIterator<'a, 'b>;
    type Item = u8;

    fn into_iter(self) -> Self::IntoIter {
        PatchIterator {
            patch: self,
            idx: 0,
        }
    }
}

pub struct PatchIterator<'a, 'b> {
    patch: &'b Patch<'a>,
    idx: usize,
}

impl<'a, 'b> Iterator for PatchIterator<'a, 'b> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx < self.patch.base_rom.len() { // assume base rom and patched rom are the same length
            let byte = self.patch[self.idx];
            self.idx += 1;
            Some(byte)
        } else {
            None
        }
    }
}

impl FusedIterator for PatchIterator<'_, '_> {}

pub fn diff_roms<'a>(base_rom: &'a [u8], patched_rom: &'a [u8]) -> Patch<'a> {
    Patch {
        base_rom,
        changed_segments: vec![(0, Cow::Borrowed(patched_rom))],
    }
}
