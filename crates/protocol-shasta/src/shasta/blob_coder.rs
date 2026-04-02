#![allow(
    clippy::cast_possible_truncation,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::trivially_copy_pass_by_ref
)]

//! Blob sidecar coder compatible with the Shasta/Kona blob encoding scheme.

use alloy_eips::eip4844::{
    BYTES_PER_BLOB, Blob, VERSIONED_HASH_VERSION_KZG,
    builder::{PartialSidecar, SidecarCoder},
    utils::WholeFe,
};
use std::vec::Vec;

const BLOB_ENCODING_VERSION: u8 = 0;
const BLOB_MAX_DATA_SIZE: usize = (4 * 31 + 3) * 1024 - 4;
const BLOB_ENCODING_ROUNDS: usize = 1024;
const ROUND_BYTES: usize = 4 * 31 + 3;
const FIRST_ROUND_CAPACITY: usize = 27 + 3 + 3 * 31;

/// Shasta blob coder copied from taiko-mono-rs so guest/proof code can decode raw blobs without
/// enabling network-only protocol features.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlobCoder;

fn ensure_field_element(field_elements: &mut Vec<[u8; 32]>, index: usize) -> &mut [u8; 32] {
    if index >= field_elements.len() {
        field_elements.push([0u8; 32]);
    }
    &mut field_elements[index]
}

fn process_chunk(chunk: &[u8], builder: &mut PartialSidecar, required_fe_count: usize) {
    let mut read_offset = 0usize;
    let mut field_elements = Vec::with_capacity(required_fe_count);
    let mut write_offset = 0usize;

    let write1 = |value: u8, write_offset: &mut usize, field_elements: &mut Vec<[u8; 32]>| {
        assert!(value & 0b1100_0000 == 0, "BlobCoder: invalid 6-bit value");
        assert_eq!(*write_offset % 32, 0);
        let fe_index = *write_offset / 32;
        let fe = ensure_field_element(field_elements, fe_index);
        fe[0] = value;
        *write_offset += 1;
    };

    let write31 = |buf: &[u8; 31], write_offset: &mut usize, field_elements: &mut Vec<[u8; 32]>| {
        assert_eq!(*write_offset % 32, 1);
        let fe_index = *write_offset / 32;
        let fe = ensure_field_element(field_elements, fe_index);
        fe[1..].copy_from_slice(buf);
        *write_offset += 31;
    };

    let read1 = |chunk: &[u8], read_offset: &mut usize| -> u8 {
        if *read_offset >= chunk.len() {
            0
        } else {
            let value = chunk[*read_offset];
            *read_offset += 1;
            value
        }
    };

    let read31 = |buf: &mut [u8; 31], chunk: &[u8], read_offset: &mut usize| {
        if *read_offset >= chunk.len() {
            buf.fill(0);
        } else {
            let remaining = chunk.len() - *read_offset;
            let to_copy = remaining.min(31);
            buf[..to_copy].copy_from_slice(&chunk[*read_offset..*read_offset + to_copy]);
            buf[to_copy..].fill(0);
            *read_offset += to_copy;
        }
    };

    let mut buf31 = [0u8; 31];

    for round in 0..BLOB_ENCODING_ROUNDS {
        if read_offset >= chunk.len() {
            break;
        }

        if round == 0 {
            buf31.fill(0);
            buf31[0] = BLOB_ENCODING_VERSION;
            let length = chunk.len() as u32;
            buf31[1] = ((length >> 16) & 0xff) as u8;
            buf31[2] = ((length >> 8) & 0xff) as u8;
            buf31[3] = (length & 0xff) as u8;
            let available = chunk.len() - read_offset;
            let to_copy = available.min(27);
            buf31[4..4 + to_copy].copy_from_slice(&chunk[read_offset..read_offset + to_copy]);
            buf31[4 + to_copy..].fill(0);
            read_offset += to_copy;
        } else {
            read31(&mut buf31, chunk, &mut read_offset);
        }

        let x = read1(chunk, &mut read_offset);
        let a = x & 0b0011_1111;
        write1(a, &mut write_offset, &mut field_elements);
        write31(&buf31, &mut write_offset, &mut field_elements);

        read31(&mut buf31, chunk, &mut read_offset);
        let y = read1(chunk, &mut read_offset);
        let b = (y & 0b0000_1111) | ((x & 0b1100_0000) >> 2);
        write1(b, &mut write_offset, &mut field_elements);
        write31(&buf31, &mut write_offset, &mut field_elements);

        read31(&mut buf31, chunk, &mut read_offset);
        let z = read1(chunk, &mut read_offset);
        let c = z & 0b0011_1111;
        write1(c, &mut write_offset, &mut field_elements);
        write31(&buf31, &mut write_offset, &mut field_elements);

        read31(&mut buf31, chunk, &mut read_offset);
        let d = ((z & 0b1100_0000) >> 2) | ((y & 0b1111_0000) >> 4);
        write1(d, &mut write_offset, &mut field_elements);
        write31(&buf31, &mut write_offset, &mut field_elements);
    }

    assert!(
        read_offset >= chunk.len(),
        "BlobCoder: payload did not fit into a single blob"
    );

    for fe in &field_elements {
        let whole = WholeFe::new(&fe[..])
            .expect("encoded field element must have the top two bits cleared");
        builder.ingest_valid_fe(whole);
    }
}

impl BlobCoder {
    /// Decode all blobs using the Kona-compatible scheme.
    #[must_use]
    pub fn decode_blobs(blobs: &[Blob]) -> Option<Vec<Vec<u8>>> {
        let mut coder = Self;
        coder.decode_all(blobs)
    }
}

impl SidecarCoder for BlobCoder {
    fn required_fe(&self, data: &[u8]) -> usize {
        if data.is_empty() {
            return 0;
        }

        let mut remaining = data.len();
        let mut fes = 4usize;
        remaining = remaining.saturating_sub(FIRST_ROUND_CAPACITY);

        while remaining > 0 {
            fes += 4;
            remaining = remaining.saturating_sub(ROUND_BYTES);
        }

        fes
    }

    fn code(&mut self, builder: &mut PartialSidecar, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        for chunk in data.chunks(BLOB_MAX_DATA_SIZE) {
            process_chunk(chunk, builder, self.required_fe(chunk));
        }
    }

    fn finish(self, _builder: &mut PartialSidecar) {}

    fn decode_all(&mut self, blobs: &[Blob]) -> Option<Vec<Vec<u8>>> {
        if blobs.is_empty() {
            return None;
        }

        let mut results = Vec::with_capacity(blobs.len());
        for blob in blobs {
            let decoded = decode_blob(blob)?;
            results.push(decoded);
        }
        Some(results)
    }
}

fn decode_field_element_bytes(
    data: &[u8],
    output: &mut [u8],
    output_pos: usize,
    input_pos: usize,
) -> Option<(u8, usize, usize)> {
    if input_pos + 32 > data.len() || output_pos + 31 > output.len() {
        return None;
    }

    let header = data[input_pos];
    if header & 0b1100_0000 != 0 {
        return None;
    }

    output[output_pos..output_pos + 31].copy_from_slice(&data[input_pos + 1..input_pos + 32]);
    Some((header, output_pos + 32, input_pos + 32))
}

fn reassemble_encoded_bytes(
    mut output_pos: usize,
    encoded_byte: &[u8; 4],
    output: &mut [u8],
) -> usize {
    output_pos -= 1;
    let x = (encoded_byte[0] & 0b0011_1111) | ((encoded_byte[1] & 0b0011_0000) << 2);
    let y = (encoded_byte[1] & 0b0000_1111) | ((encoded_byte[3] & 0b0000_1111) << 4);
    let z = (encoded_byte[2] & 0b0011_1111) | ((encoded_byte[3] & 0b0011_0000) << 2);
    output[output_pos - 32] = z;
    output[output_pos - (32 * 2)] = y;
    output[output_pos - (32 * 3)] = x;
    output_pos
}

fn decode_blob(blob: &Blob) -> Option<Vec<u8>> {
    let data: &[u8] = blob.as_ref();

    if data[VERSIONED_HASH_VERSION_KZG as usize] != BLOB_ENCODING_VERSION {
        return None;
    }

    let length = u32::from_be_bytes([0, data[2], data[3], data[4]]) as usize;
    if length > BLOB_MAX_DATA_SIZE {
        return None;
    }

    let mut output = vec![0u8; BLOB_MAX_DATA_SIZE];
    output[0..27].copy_from_slice(&data[5..32]);

    let mut output_pos = 28usize;
    let mut input_pos = 32usize;
    let mut encoded_byte = [0u8; 4];
    encoded_byte[0] = data[0];

    for b in encoded_byte.iter_mut().skip(1) {
        let (encoded, new_output_pos, new_input_pos) =
            decode_field_element_bytes(data, &mut output, output_pos, input_pos)?;
        *b = encoded;
        output_pos = new_output_pos;
        input_pos = new_input_pos;
    }

    output_pos = reassemble_encoded_bytes(output_pos, &encoded_byte, &mut output);

    while output_pos < length && input_pos < BYTES_PER_BLOB {
        for byte in &mut encoded_byte {
            let (encoded, new_output_pos, new_input_pos) =
                decode_field_element_bytes(data, &mut output, output_pos, input_pos)?;
            *byte = encoded;
            output_pos = new_output_pos;
            input_pos = new_input_pos;
        }
        output_pos = reassemble_encoded_bytes(output_pos, &encoded_byte, &mut output);
    }

    if output_pos < length {
        return None;
    }

    if data[input_pos..].iter().any(|&byte| byte != 0) {
        return None;
    }

    output.truncate(length);
    Some(output)
}
