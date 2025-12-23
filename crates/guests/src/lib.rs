#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Guest program ELF assets for Raiko2.

pub mod risc0 {
    pub mod shasta {
        pub const PROPOSAL_ELF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/elf/risc0_shasta_proposal.elf"
        ));
        pub const AGGREGATION_ELF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/elf/risc0_shasta_aggregation.elf"
        ));
    }
}

pub mod sp1 {
    pub mod shasta {
        pub const PROPOSAL_ELF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/elf/sp1_shasta_proposal.elf"
        ));
        pub const AGGREGATION_ELF: &[u8] = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/elf/sp1_shasta_aggregation.elf"
        ));
    }
}
