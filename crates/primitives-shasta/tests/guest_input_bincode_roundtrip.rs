//! Regression test: `test.json` GuestInput must bincode roundtrip.
use raiko2_primitives_shasta::GuestInput;

#[test]
fn guest_input_json_to_bincode_roundtrip_test_json() -> Result<(), Box<dyn std::error::Error>> {
    // Keep this test pinned to the repo's `test.json` so we catch any newly introduced
    // non-bincode-compatible custom serde early (before SP1 runtime).
    let json = include_str!("../../../test.json");
    let input: GuestInput = serde_json::from_str(json)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    // First, isolate the problematic region: Taiko manifest's proposal_event.
    // This makes failures much faster to debug than full GuestInput.
    {
        let ev = &input.taiko.proposal_event;
        let ev_bytes = bincode::serialize(ev)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let _: raiko2_protocol_shasta::shasta::ShastaEventData = bincode::deserialize(&ev_bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    }
    {
        let w_bytes = bincode::serialize(&input.witnesses)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let _: Vec<raiko2_primitives::StatelessInput> = bincode::deserialize(&w_bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    }
    {
        let t_bytes = bincode::serialize(&input.taiko)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        use std::io::{Cursor, Read};

        struct CountingReader<R> {
            inner: R,
            pos: usize,
        }

        impl<R> CountingReader<R> {
            fn new(inner: R) -> Self {
                Self { inner, pos: 0 }
            }
        }

        impl<R: Read> Read for CountingReader<R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.inner.read(buf)?;
                self.pos += n;
                Ok(n)
            }
        }

        let mut r = CountingReader::new(Cursor::new(t_bytes.as_slice()));
        match bincode::deserialize_from::<_, raiko2_protocol_shasta::TaikoManifest>(&mut r) {
            Ok(_) => {}
            Err(e) => {
                eprintln!(
                    "[guest_input_bincode_roundtrip] TaikoManifest bincode deserialize failed at byte {}: {e}",
                    r.pos
                );
                explain_taiko_manifest_offset(&input.taiko, r.pos);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("bincode deserialize GuestInput.taiko: {e}"),
                )
                .into());
            }
        }
    }

    let bytes = bincode::serialize(&input)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let decoded: GuestInput = {
        use std::io::{Cursor, Read};

        struct CountingReader<R> {
            inner: R,
            pos: usize,
        }

        impl<R> CountingReader<R> {
            fn new(inner: R) -> Self {
                Self { inner, pos: 0 }
            }
        }

        impl<R: Read> Read for CountingReader<R> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                let n = self.inner.read(buf)?;
                self.pos += n;
                Ok(n)
            }
        }

        let mut r = CountingReader::new(Cursor::new(bytes.as_slice()));
        match bincode::deserialize_from::<_, GuestInput>(&mut r) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "[guest_input_bincode_roundtrip] deserialize failed at byte {}: {e}",
                    r.pos
                );
                explain_guest_input_offset(&input, r.pos);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!(
                        "bincode deserialize GuestInput failed at byte {}: {e}",
                        r.pos
                    ),
                )
                .into());
            }
        }
    };

    // Ensure the encoding is stable (helps catch accidental config-dependent encoding).
    let bytes2 = bincode::serialize(&decoded)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    assert_eq!(bytes2, bytes, "bincode bytes changed after roundtrip");
    Ok(())
}

fn explain_taiko_manifest_offset(m: &raiko2_protocol_shasta::TaikoManifest, off: usize) {
    let mut cursor = 0usize;

    let proposal_id_len = bincode::serialize(&m.proposal_id)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + proposal_id_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.proposal_id at off={}",
            off - cursor
        );
        return;
    }
    cursor += proposal_id_len;

    let l1_header_len = bincode::serialize(&m.l1_header)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + l1_header_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.l1_header at off={}",
            off - cursor
        );
        return;
    }
    cursor += l1_header_len;

    let proposal_event_len = bincode::serialize(&m.proposal_event)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + proposal_event_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.proposal_event at off={}",
            off - cursor
        );
        return;
    }
    cursor += proposal_event_len;

    let chain_spec_len = bincode::serialize(&m.chain_spec)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + chain_spec_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.chain_spec at off={}",
            off - cursor
        );
        return;
    }
    cursor += chain_spec_len;

    let prover_data_len = bincode::serialize(&m.prover_data)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + prover_data_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.prover_data at off={}",
            off - cursor
        );
        return;
    }
    cursor += prover_data_len;

    let data_sources_len = bincode::serialize(&m.data_sources)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + data_sources_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.data_sources at off={}",
            off - cursor
        );
        return;
    }

    eprintln!(
        "[guest_input_bincode_roundtrip] offset {off} is past TaikoManifest end (computed_end={})",
        cursor + data_sources_len
    );
}

fn explain_guest_input_offset(input: &GuestInput, fail_off: usize) {
    // Best-effort mapping of the bincode offset into a field path by re-serializing sub-structures.
    // This keeps the debugging loop fast without having to run SP1.
    let witnesses_len = match bincode::serialize(&input.witnesses) {
        Ok(b) => b.len(),
        Err(e) => {
            eprintln!("[guest_input_bincode_roundtrip] cannot serialize witnesses: {e}");
            return;
        }
    };

    if fail_off < witnesses_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] fail_off={} is within GuestInput.witnesses (len={witnesses_len})",
            fail_off
        );
        return;
    }

    let taiko_off = fail_off - witnesses_len;
    eprintln!(
        "[guest_input_bincode_roundtrip] fail_off={} is within GuestInput.taiko at taiko_off={taiko_off}",
        fail_off
    );

    // TaikoManifest field order (see raiko2_protocol::manifest::TaikoManifest)
    let mut cursor = 0usize;

    let proposal_id_len = bincode::serialize(&input.taiko.proposal_id)
        .map(|b| b.len())
        .unwrap_or(0);
    if taiko_off < cursor + proposal_id_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.proposal_id at off={}",
            taiko_off - cursor
        );
        return;
    }
    cursor += proposal_id_len;

    let l1_header_len = bincode::serialize(&input.taiko.l1_header)
        .map(|b| b.len())
        .unwrap_or(0);
    if taiko_off < cursor + l1_header_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.l1_header at off={}",
            taiko_off - cursor
        );
        return;
    }
    cursor += l1_header_len;

    let proposal_event_len = bincode::serialize(&input.taiko.proposal_event)
        .map(|b| b.len())
        .unwrap_or(0);
    if taiko_off < cursor + proposal_event_len {
        let event_off = taiko_off - cursor;
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.proposal_event at off={event_off}"
        );
        explain_shasta_event_data_offset(&input.taiko.proposal_event, event_off);
        return;
    }
    cursor += proposal_event_len;

    let chain_spec_len = bincode::serialize(&input.taiko.chain_spec)
        .map(|b| b.len())
        .unwrap_or(0);
    if taiko_off < cursor + chain_spec_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.chain_spec at off={}",
            taiko_off - cursor
        );
        return;
    }
    cursor += chain_spec_len;

    let prover_data_len = bincode::serialize(&input.taiko.prover_data)
        .map(|b| b.len())
        .unwrap_or(0);
    if taiko_off < cursor + prover_data_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.prover_data at off={}",
            taiko_off - cursor
        );
        return;
    }
    cursor += prover_data_len;

    let data_sources_len = bincode::serialize(&input.taiko.data_sources)
        .map(|b| b.len())
        .unwrap_or(0);
    if taiko_off < cursor + data_sources_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in TaikoManifest.data_sources at off={}",
            taiko_off - cursor
        );
        return;
    }

    eprintln!(
        "[guest_input_bincode_roundtrip] taiko_off={taiko_off} is past TaikoManifest end (computed_end={})",
        cursor + data_sources_len
    );
}

fn explain_shasta_event_data_offset(
    e: &raiko2_protocol_shasta::shasta::ShastaEventData,
    event_off: usize,
) {
    let proposal_len = bincode::serialize(&e.proposal)
        .map(|b| b.len())
        .unwrap_or(0);
    if event_off < proposal_len {
        eprintln!("[guest_input_bincode_roundtrip] in ShastaEventData.proposal at off={event_off}");
        explain_shasta_proposal_offset(&e.proposal, event_off);
        return;
    }
    eprintln!(
        "[guest_input_bincode_roundtrip] event_off={event_off} is past ShastaEventData end (computed_end={proposal_len})"
    );
}

fn explain_shasta_proposal_offset(p: &raiko2_protocol_shasta::shasta::Proposal, off: usize) {
    let mut cursor = 0usize;

    let id_len = bincode::serialize(&p.id).map(|b| b.len()).unwrap_or(0);
    if off < cursor + id_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in Proposal.id at off={}",
            off - cursor
        );
        return;
    }
    cursor += id_len;

    let ts_len = bincode::serialize(&p.timestamp)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + ts_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in Proposal.timestamp at off={}",
            off - cursor
        );
        return;
    }
    cursor += ts_len;

    let end_ts_len = bincode::serialize(&p.endOfSubmissionWindowTimestamp)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + end_ts_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in Proposal.endOfSubmissionWindowTimestamp at off={}",
            off - cursor
        );
        return;
    }
    cursor += end_ts_len;

    let proposer_len = bincode::serialize(&p.proposer)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + proposer_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in Proposal.proposer at off={}",
            off - cursor
        );
        return;
    }
    cursor += proposer_len;

    let parent_len = bincode::serialize(&p.parentProposalHash)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + parent_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in Proposal.parentProposalHash at off={}",
            off - cursor
        );
        return;
    }
    cursor += parent_len;

    let origin_number_len = bincode::serialize(&p.originBlockNumber)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + origin_number_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in Proposal.originBlockNumber at off={}",
            off - cursor
        );
        return;
    }
    cursor += origin_number_len;

    let origin_hash_len = bincode::serialize(&p.originBlockHash)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + origin_hash_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in Proposal.originBlockHash at off={}",
            off - cursor
        );
        return;
    }
    cursor += origin_hash_len;

    let basefee_len = bincode::serialize(&p.basefeeSharingPctg)
        .map(|b| b.len())
        .unwrap_or(0);
    if off < cursor + basefee_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in Proposal.basefeeSharingPctg at off={}",
            off - cursor
        );
        return;
    }
    cursor += basefee_len;

    let sources_len = bincode::serialize(&p.sources).map(|b| b.len()).unwrap_or(0);
    if off < cursor + sources_len {
        eprintln!(
            "[guest_input_bincode_roundtrip] in Proposal.sources at off={}",
            off - cursor
        );
        return;
    }

    eprintln!(
        "[guest_input_bincode_roundtrip] off={off} is past Proposal end (computed_end={})",
        cursor + sources_len
    );
}
