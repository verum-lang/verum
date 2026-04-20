//! Isabelle → Verum theorem importer CLI.
//!
//! Usage:
//!
//! ```bash
//! verum-isabelle-import --input Graph_Library.thy --output out/
//! verum-isabelle-import -i Graph_Library.thy -o out/ --dry-run
//! ```
//!
//! `--dry-run` prints each imported theorem's emitted `.vr` text to
//! stdout instead of writing files. Useful for spot-checking the
//! translation before committing files.

use std::process::ExitCode;

use isabelle_graph_import::{emit_verum, parse_theory, write_out_dir};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut dry_run = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage();
                return ExitCode::SUCCESS;
            }
            "-i" | "--input" => {
                i += 1;
                input = args.get(i).cloned();
            }
            "-o" | "--output" => {
                i += 1;
                output = args.get(i).cloned();
            }
            "--dry-run" => dry_run = true,
            other => {
                eprintln!("error: unknown argument `{other}`");
                print_usage();
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    let Some(input_path) = input else {
        eprintln!("error: --input <path.thy> is required");
        print_usage();
        return ExitCode::from(2);
    };

    let src = match std::fs::read_to_string(&input_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read `{input_path}`: {e}");
            return ExitCode::from(1);
        }
    };

    let theorems = parse_theory(&src);
    eprintln!(
        "isabelle-import: parsed {} theorem(s) from {}",
        theorems.len(),
        input_path
    );

    if dry_run {
        for th in &theorems {
            println!("// ---- {} ----", th.name);
            print!("{}", emit_verum(th));
        }
        return ExitCode::SUCCESS;
    }

    let Some(output_dir) = output else {
        eprintln!("error: --output <dir> is required (or pass --dry-run)");
        print_usage();
        return ExitCode::from(2);
    };

    match write_out_dir(&theorems, std::path::Path::new(&output_dir)) {
        Ok(paths) => {
            for p in &paths {
                eprintln!("  wrote {}", p.display());
            }
            eprintln!(
                "isabelle-import: wrote {} file(s) under {}",
                paths.len(),
                output_dir
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: write failed: {e}");
            ExitCode::from(1)
        }
    }
}

fn print_usage() {
    eprintln!(
        "verum-isabelle-import: import Isabelle/HOL Graph_Library theorems\n\
         \n\
         USAGE:\n\
             verum-isabelle-import -i <FILE.thy> -o <OUT_DIR>\n\
             verum-isabelle-import -i <FILE.thy> --dry-run\n\
         \n\
         OPTIONS:\n\
             -i, --input   <FILE>  Isabelle .thy source (required)\n\
             -o, --output  <DIR>   Destination dir for emitted .vr files\n\
                 --dry-run         Print emitted .vr to stdout instead of writing\n\
             -h, --help            Show this help"
    );
}
