use clap::Parser;
use zkgas::{
    identity::executable_identity,
    report::{ReportArgs, run_report},
};

fn main() {
    let args = match ReportArgs::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let code = if error.use_stderr() { 1 } else { 0 };
            let _ = error.print();
            std::process::exit(code);
        }
    };
    let code = match executable_identity().and_then(|executable| run_report(&args, &executable)) {
        Ok(run) => {
            println!("{}", run.report.report_id);
            run.exit_code()
        }
        Err(error) => {
            eprintln!("{error:#}");
            1
        }
    };
    std::process::exit(code);
}
