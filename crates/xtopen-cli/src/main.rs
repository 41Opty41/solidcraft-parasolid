use std::fs;
use std::path::Path;
use std::process;
use std::time::Instant;

use xtopen_core::{
    convert_bytes_to_step, dumps_step, extract_prt, parse_and_extract_brep, xb_to_xt, xt_to_xb,
    PrtStreamKind, StepOptions, StepSchema, StepUnits,
};

fn usage() -> ! {
    eprintln!("Usage:");
    eprintln!("  xtopen-cli convert <input.prt|x_b|x_t> <output.step> [--units mm|m] [--schema AP214|AP242]");
    eprintln!("  xtopen-cli transcode <input.x_b|x_t> <output.x_t|x_b>");
    eprintln!("  xtopen-cli inspect <input.prt|x_b|x_t>");
    eprintln!("  xtopen-cli bench <input.prt|x_b|x_t> [--iters N]");
    eprintln!("  xtopen-cli extract-prt <input.prt> <out_dir>");
    process::exit(2);
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        sorted[n / 2]
    } else {
        0.5 * (sorted[n / 2 - 1] + sorted[n / 2])
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        usage();
    }
    match args[1].as_str() {
        "convert" => {
            if args.len() < 4 {
                usage();
            }
            let input = Path::new(&args[2]);
            let output = Path::new(&args[3]);
            let mut opts = StepOptions::default();
            if let Some(stem) = input.file_stem().and_then(|s| s.to_str()) {
                opts.name = stem.to_string();
            }
            let mut i = 4;
            while i < args.len() {
                match args[i].as_str() {
                    "--units" => {
                        i += 1;
                        let u = args.get(i).map(String::as_str).unwrap_or_else(|| usage());
                        opts.units = match u {
                            "mm" => StepUnits::Mm,
                            "m" => StepUnits::M,
                            _ => usage(),
                        };
                    }
                    "--schema" => {
                        i += 1;
                        let s = args.get(i).map(String::as_str).unwrap_or_else(|| usage());
                        opts.schema = match s.to_ascii_uppercase().as_str() {
                            "AP214" => StepSchema::Ap214,
                            "AP242" => StepSchema::Ap242,
                            _ => usage(),
                        };
                    }
                    _ => usage(),
                }
                i += 1;
            }
            let data = fs::read(input).unwrap_or_else(|e| {
                eprintln!("Failed to read {}: {e}", input.display());
                process::exit(1);
            });
            let res = convert_bytes_to_step(&data, &opts).unwrap_or_else(|e| {
                eprintln!("Conversion error: {e}");
                process::exit(1);
            });
            fs::write(output, &res.step).unwrap_or_else(|e| {
                eprintln!("Failed to write {}: {e}", output.display());
                process::exit(1);
            });
            println!(
                "Converted {} -> {} ({} bodies, {} faces, {} bytes STEP)",
                input.display(),
                output.display(),
                res.bodies,
                res.faces,
                res.step.len()
            );
        }
        "transcode" => {
            if args.len() < 4 {
                usage();
            }
            let input = Path::new(&args[2]);
            let output = Path::new(&args[3]);
            let data = fs::read(input).unwrap_or_else(|e| {
                eprintln!("Failed to read {}: {e}", input.display());
                process::exit(1);
            });
            let to_xt = matches!(
                output.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()).as_deref(),
                Some("x_t" | "xt" | "xmt_txt")
            );
            let res = if to_xt {
                xb_to_xt(&data)
            } else {
                xt_to_xb(&data)
            }
            .unwrap_or_else(|e| {
                eprintln!("Transcode error: {e}");
                process::exit(1);
            });
            fs::write(output, &res).unwrap_or_else(|e| {
                eprintln!("Failed to write {}: {e}", output.display());
                process::exit(1);
            });
            println!(
                "Transcoded {} -> {} ({} bytes)",
                input.display(),
                output.display(),
                res.len()
            );
        }
        "inspect" => {
            let input = Path::new(&args[2]);
            let data = fs::read(input).unwrap_or_else(|e| {
                eprintln!("Failed to read {}: {e}", input.display());
                process::exit(1);
            });
            let model = parse_and_extract_brep(&data).unwrap_or_else(|e| {
                eprintln!("Parse error: {e}");
                process::exit(1);
            });
            println!(
                "{{\"format\":\"{}\",\"schema\":\"{}\",\"bodies\":{},\"faces\":{},\"edges\":{},\"vertices\":{},\"warnings\":{}}}",
                model.header.format_kind,
                model.header.schema_name,
                model.bodies.len(),
                model.faces.len(),
                model.edges.len(),
                model.vertices.len(),
                model.warnings.len(),
            );
        }
        "extract-prt" => {
            if args.len() < 4 {
                usage();
            }
            let input = Path::new(&args[2]);
            let out_dir = Path::new(&args[3]);
            fs::create_dir_all(out_dir).unwrap_or_else(|e| {
                eprintln!("Failed to create {}: {e}", out_dir.display());
                process::exit(1);
            });
            let data = fs::read(input).unwrap_or_else(|e| {
                eprintln!("Failed to read {}: {e}", input.display());
                process::exit(1);
            });
            let archive = extract_prt(&data).unwrap_or_else(|e| {
                eprintln!("Extract error: {e}");
                process::exit(1);
            });
            let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("part");
            for st in &archive.streams {
                let ext = match st.kind {
                    PrtStreamKind::Delta => "x_d",
                    _ => "x_b",
                };
                let p = out_dir.join(format!("{stem}_stream{:02}.{ext}", st.index));
                fs::write(&p, &st.standalone_xb).unwrap_or_else(|e| {
                    eprintln!("Failed to write {}: {e}", p.display());
                    process::exit(1);
                });
            }
            if let Some(ref prev) = archive.preview {
                let ext = if prev.starts_with(b"\x89PNG") { "png" } else { "jpg" };
                let p = out_dir.join(format!("{stem}_preview.{ext}"));
                let _ = fs::write(&p, prev);
            }
            println!(
                "Extracted {} stream(s) from {} ({})",
                archive.streams.len(),
                input.display(),
                archive.container.label()
            );
        }
        "bench" => {
            let input = Path::new(&args[2]);
            let mut iters = 50usize;
            let mut i = 3;
            while i < args.len() {
                if args[i] == "--iters" {
                    i += 1;
                    iters = args
                        .get(i)
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or_else(|| usage())
                        .max(1);
                } else {
                    usage();
                }
                i += 1;
            }
            let data = fs::read(input).unwrap_or_else(|e| {
                eprintln!("Failed to read {}: {e}", input.display());
                process::exit(1);
            });
            let mut opts = StepOptions::default();
            if let Some(stem) = input.file_stem().and_then(|s| s.to_str()) {
                opts.name = stem.to_string();
            }

            // Warmup once
            let m_warm = parse_and_extract_brep(&data).expect("bench parse failed");
            let step_warm = dumps_step(&m_warm, &opts);
            std::hint::black_box(&step_warm);

            let mut parse_ms = Vec::with_capacity(iters);
            let mut step_ms = Vec::with_capacity(iters);
            let mut total_ms = Vec::with_capacity(iters);

            for _ in 0..iters {
                let t0 = Instant::now();
                let model = parse_and_extract_brep(&data).unwrap();
                let t1 = Instant::now();
                let step = dumps_step(&model, &opts);
                let t2 = Instant::now();
                std::hint::black_box(step.len());

                parse_ms.push((t1 - t0).as_secs_f64() * 1000.0);
                step_ms.push((t2 - t1).as_secs_f64() * 1000.0);
                total_ms.push((t2 - t0).as_secs_f64() * 1000.0);
            }

            parse_ms.sort_by(|a, b| a.total_cmp(b));
            step_ms.sort_by(|a, b| a.total_cmp(b));
            total_ms.sort_by(|a, b| a.total_cmp(b));

            let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
            println!(
                "{{\"bytes\":{},\"iters\":{},\"step_bytes\":{},\"median_ms\":{{\"parse_brep\":{:.6},\"step\":{:.6},\"total\":{:.6}}},\"mean_ms\":{{\"parse_brep\":{:.6},\"step\":{:.6},\"total\":{:.6}}}}}",
                data.len(),
                iters,
                step_warm.len(),
                median(&parse_ms),
                median(&step_ms),
                median(&total_ms),
                mean(&parse_ms),
                mean(&step_ms),
                mean(&total_ms),
            );
        }
        _ => usage(),
    }
}
