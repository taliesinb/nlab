//! nlab-conv CLI: one conversion per process, source on STDIN.
//!
//!   nlab-conv page [TITLE]   whole page -> typst (NLAB_PAGEIDX / NLAB_FILES_MAP
//!                            name the link and image TSVs)
//!   nlab-conv grid | grids   array-diagram cell grid(s) as JSON
//!   nlab-conv typsts         NUL-separated formulas -> typst records
//!   nlab-conv tikzcd(s)      tikz-cd -> fletcher
//!   nlab-conv dump           mitex syntax tree
//!
//! `page` and `tikzcd(s)` emit against the local mitex fork; `grid(s)` and
//! `typsts` only with NLAB_LOCAL_MITEX=1. The same conversions are
//! available in-process from Python as `nlab.conv`.

use std::io::{Read, Write};
use std::sync::Arc;

use nlab_conv::{with_context, Context};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("grid");
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let env_local = std::env::var("NLAB_LOCAL_MITEX").is_ok_and(|v| v == "1");
    let local = matches!(mode, "page" | "tikzcd" | "tikzcds") || env_local;
    let ctx = Arc::new(Context::from_env());
    with_context(&ctx, local, || run(mode, &args, &input));
}

fn run(mode: &str, args: &[String], input: &str) {
    let out = std::io::stdout();
    let mut out = out.lock();
    match mode {
        "dump" => print!("{}", nlab_conv::dump_tree(input)),
        "grid" => match nlab_conv::formula_grid_json(input) {
            Ok(json) => println!("{json}"),
            Err(status) => {
                eprintln!("{status}");
                std::process::exit(3);
            }
        },
        "grids" => {
            for rec in input.split('\u{0}').filter(|r| !r.is_empty()) {
                match nlab_conv::formula_grid_json(rec) {
                    Ok(json) => write!(out, "ok\u{1f}{json}\u{0}").unwrap(),
                    Err(status) => write!(out, "{status}\u{1f}\u{0}").unwrap(),
                }
            }
        }
        "page" => {
            let title = args.get(2).map(String::as_str);
            write!(out, "{}", nlab_conv::page_to_typst(input, title)).unwrap();
        }
        "tikzcd" => match nlab_conv::tikzcd_to_fletcher(input) {
            Ok((code, warns)) => {
                for w in warns {
                    eprintln!("warn: ignored option {w:?}");
                }
                writeln!(out, "{code}").unwrap();
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(3);
            }
        },
        "tikzcds" => {
            for rec in input.split('\u{0}').filter(|r| !r.trim().is_empty()) {
                match nlab_conv::tikzcd_to_fletcher(rec) {
                    Ok((code, warns)) => {
                        write!(out, "ok\u{1f}{}\u{1f}{}\u{0}", code, warns.join("\u{1e}")).unwrap()
                    }
                    Err(e) => write!(out, "err:{e}\u{1f}\u{1f}\u{0}").unwrap(),
                }
            }
        }
        "typsts" => {
            for rec in input.split('\u{0}').filter(|r| !r.is_empty()) {
                let (class, status, code) = nlab_conv::formula_to_typst(rec);
                write!(out, "{status}\u{1f}{class}\u{1f}{}\u{0}", code.unwrap_or_default())
                    .unwrap();
            }
        }
        m => {
            eprintln!("unknown mode: {m}");
            std::process::exit(2);
        }
    }
}
