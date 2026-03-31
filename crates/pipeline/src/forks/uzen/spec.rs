//! Uzen fork uses the same pipeline logic as Shasta (`GuestInput`, `shasta_*` manifest keys).

pub type UzenSpec<Pr, Bk, Pv> = crate::forks::shasta::ShastaSpec<Pr, Bk, Pv>;
