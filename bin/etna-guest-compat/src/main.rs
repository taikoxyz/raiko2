//! Execute the Etna compatibility fixture and compare raw public bytes with the native run.

const STOCK_JOURNAL: &[u8] =
    include_bytes!("../../../crates/etna-guest-compat/fixtures/journal.bin");

fn main() {
    let mut args = std::env::args_os().skip(1);
    let backend = args.next().unwrap_or_else(|| "native".into());
    let native = etna_guest_compat_fixture::run();
    assert_eq!(
        native.as_slice(),
        STOCK_JOURNAL,
        "native diagnostic differs from the stock oracle"
    );

    let journal = match (backend.to_str(), args.next(), args.next()) {
        (Some("native"), None, None) => native.clone(),
        #[cfg(feature = "risc0")]
        (Some("risc0"), Some(path), None) => {
            let elf = std::fs::read(path).expect("read RISC Zero diagnostic ELF");
            execute_risc0(&elf)
        }
        #[cfg(feature = "sp1")]
        (Some("sp1"), Some(path), None) => {
            let elf = std::fs::read(path).expect("read SP1 diagnostic ELF");
            execute_sp1(&elf)
        }
        _ => {
            eprintln!("Usage: etna-guest-compat [native | risc0 ELF | sp1 ELF]");
            eprintln!("Enable the corresponding Cargo feature for guest execution.");
            std::process::exit(2);
        }
    };
    assert_eq!(journal, native, "guest journal differs from native");
    println!(
        "{} journal {}",
        backend.to_string_lossy(),
        hex::encode(journal)
    );
}

#[cfg(feature = "risc0")]
fn execute_risc0(elf: &[u8]) -> Vec<u8> {
    // The target outputs a raw ELF. The executor takes a ProgramBinary with the V1 kernel.
    let program = risc0_binfmt::ProgramBinary::new(elf, risc0_zkos_v1compat::V1COMPAT_ELF).encode();
    let env = risc0_zkvm::ExecutorEnv::builder()
        .build()
        .expect("build RISC Zero executor input");
    let session = risc0_zkvm::default_executor()
        .execute(env, &program)
        .expect("execute RISC Zero ELF");
    assert_eq!(session.exit_code, risc0_zkvm::ExitCode::Halted(0));
    println!(
        "RISC Zero CPU execution {} cycles, {} segments",
        session.cycles(),
        session.segments.len()
    );
    session.journal.bytes
}

#[cfg(feature = "sp1")]
fn execute_sp1(elf: &[u8]) -> Vec<u8> {
    let program = sp1_core_executor::Program::from(elf).expect("decode SP1 ELF");
    let mut vm = sp1_core_executor::MinimalExecutorEnum::new(
        std::sync::Arc::new(program),
        false,
        Some(1 << 20),
    );
    while vm.try_execute_chunk().expect("execute SP1 ELF").is_some() {}
    assert_eq!(vm.exit_code(), 0);
    println!("SP1 CPU execution {} cycles", vm.global_clk());
    vm.public_values_stream().clone()
}
