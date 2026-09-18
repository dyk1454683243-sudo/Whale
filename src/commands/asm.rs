use std::fs;
use std::process;
use std::time::Instant;

use assembler::isa::amd64::parser::parse;
use assembler::tokens::tokenize;
use assembler::{assemble, isa::AMD64, AssemblerOutput};

use object::{
    ObjectFile, ObjectFormat, ObjectRelocation, ObjectSymbol, RelocKind as ObjectRelocKind,
    SectionKind, SymbolBinding, SymbolVisibility,
};

pub fn run(args: Vec<String>) {
    if args.is_empty() {
        print_help();
        return;
    }

    let mut arch = None;
    let mut input = None;
    let mut output = None;

    let mut debug_mode = false;
    let mut show_ast = false;
    let mut show_token = false;
    let mut show_bytes = false;
    let mut dump_hex = false;
    let mut dump_bin = false;
    let mut dump_json = false;
    let mut _no_color = false;
    let mut _no_warn_ext = false;
    let mut show_stats = false;
    let mut trace_enable = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--help" => {
                print_help();
                return;
            }

            "--amd64" => arch = Some("amd64"),
            "--aarch64" => arch = Some("aarch64"),

            "-o" => {
                if i + 1 < args.len() {
                    output = Some(args[i + 1].clone());
                    i += 1;
                }
            }

            "--debug-whale" => debug_mode = true,
            "--ast" => show_ast = true,
            "--token" => show_token = true,
            "--bytes" => show_bytes = true,
            "--dump-hex" => dump_hex = true,
            "--dump-bin" => dump_bin = true,
            "--dump-json" => dump_json = true,
            "--no-color" => _no_color = true,
            "--no-warn-extension" => _no_warn_ext = true,
            "--stats" => show_stats = true,
            "--trace" => trace_enable = true,

            s if input.is_none() && !s.starts_with('-') => input = Some(s.to_string()),
            _ => {}
        }
        i += 1;
    }

    let arch = arch.unwrap_or_else(|| {
        eprintln!("Error: architecture must be specified (--amd64)");
        process::exit(1);
    });

    if arch != "amd64" {
        eprintln!("Error: only --amd64 is supported right now");
        process::exit(1);
    }

    let input = input.unwrap_or_else(|| {
        eprintln!("Error: missing input file.");
        process::exit(1);
    });

    let output = output.unwrap_or_else(|| {
        eprintln!("Error: missing output (-o)");
        process::exit(1);
    });

    if !output.ends_with(".o") {
        eprintln!("Error: for now, asm output must be .o (object).");
        process::exit(1);
    }

    if trace_enable {
        println!("[trace] reading input file: {}", input);
    }
    let src = fs::read_to_string(&input).unwrap_or_else(|e| {
        eprintln!("Failed to read {}: {}", input, e);
        process::exit(1);
    });

    let mut token_len: Option<usize> = None;
    let mut ast_items_len: Option<usize> = None;

    let need_tokens = debug_mode && (show_token || show_ast || show_stats);
    if need_tokens {
        if trace_enable {
            println!("[trace] tokenize start");
        }
        let tokens = tokenize(&src).unwrap_or_else(|e| {
            eprintln!("Tokenize error: {}", e);
            process::exit(1);
        });
        token_len = Some(tokens.len());

        if show_token {
            dbg!(&tokens);
        }

        if show_ast || show_stats {
            if trace_enable {
                println!("[trace] parse start");
            }
            let ast = parse(&tokens).unwrap_or_else(|e| {
                eprintln!("Parse error: {}", e);
                process::exit(1);
            });
            ast_items_len = Some(ast.items.len());

            if show_ast {
                println!("{:#?}", ast);
            }
        }
    }

    if trace_enable {
        println!("[trace] assemble start");
    }
    let start_time = Instant::now();
    let out = assemble(&src, &AMD64).unwrap_or_else(|e| {
        eprintln!("Assemble error: {}", e);
        process::exit(1);
    });
    let elapsed = start_time.elapsed();

    if trace_enable {
        println!("[trace] creating object file");
    }
    let final_bytes = build_elf_from_asm_output(&out).unwrap_or_else(|e| {
        eprintln!("ELF build error: {}", e);
        process::exit(1);
    });

    fs::write(&output, &final_bytes).unwrap_or_else(|e| {
        eprintln!("Failed to write {}: {}", output, e);
        process::exit(1);
    });

    if debug_mode && (show_bytes || dump_hex || dump_bin || dump_json) {
        dump_bytes(
            "object",
            &final_bytes,
            show_bytes,
            dump_hex,
            dump_bin,
            dump_json,
        );
    }

    if debug_mode && show_stats {
        println!(
            "== STATS ==\nTokens: {}\nAST nodes: {}\nObject bytes: {}\nTime: {} ms",
            token_len.unwrap_or(0),
            ast_items_len.unwrap_or(0),
            final_bytes.len(),
            elapsed.as_millis()
        );
    }

    println!("Wrote {} bytes to {}", final_bytes.len(), output);
}

fn build_elf_from_asm_output(out: &AssemblerOutput) -> Result<Vec<u8>, String> {
    let mut obj = ObjectFile::new(ObjectFormat::ELF64);
    let mut section_map = Vec::with_capacity(out.sections.len());

    for sec in &out.sections {
        let kind = match sec.name.as_str() {
            ".text" => SectionKind::Text,
            ".data" => SectionKind::Data,
            ".rodata" => SectionKind::ReadOnlyData,
            ".bss" => SectionKind::Bss,
            _ => SectionKind::Data,
        };

        let align = if sec.name == ".text" { 16 } else { 1 };
        let idx = obj.add_section(&sec.name, kind, align);
        obj.sections[idx].data = sec.data.clone();
        section_map.push(idx);
    }

    for sym in &out.symbols {
        let section_index = sym
            .section_index
            .and_then(|idx| section_map.get(idx).copied());

        obj.symbols.push(ObjectSymbol {
            name: sym.name.clone(),
            section_index,
            value: sym.offset as u64,
            size: 0,
            binding: if sym.is_global {
                SymbolBinding::Global
            } else {
                SymbolBinding::Local
            },
            visibility: SymbolVisibility::Default,
        });
    }

    for (asm_sec_idx, sec) in out.sections.iter().enumerate() {
        let Some(&obj_sec_idx) = section_map.get(asm_sec_idx) else {
            continue;
        };

        for reloc in &sec.relocs {
            let is_undefined_symbol = out
                .symbols
                .iter()
                .find(|s| s.name == reloc.symbol)
                .map(|s| s.section_index.is_none())
                .unwrap_or(true);

            let kind = match reloc.kind {
                assembler::assembler::RelocKind::Absolute64 => ObjectRelocKind::Absolute64,
                assembler::assembler::RelocKind::Absolute32 => ObjectRelocKind::Absolute32,
                assembler::assembler::RelocKind::Relative32 => {
                    if is_undefined_symbol {
                        ObjectRelocKind::PLT32
                    } else {
                        ObjectRelocKind::Relative32
                    }
                }
                assembler::assembler::RelocKind::Relative8 => ObjectRelocKind::Relative8,
            };

            obj.relocations.push(ObjectRelocation {
                section_index: obj_sec_idx,
                offset: reloc.offset,
                symbol: reloc.symbol.clone(),
                addend: reloc.addend,
                kind,
            });

            if obj.symbols.iter().all(|s| s.name != reloc.symbol) {
                obj.symbols.push(ObjectSymbol {
                    name: reloc.symbol.clone(),
                    section_index: None,
                    value: 0,
                    size: 0,
                    binding: SymbolBinding::Global,
                    visibility: SymbolVisibility::Default,
                });
            }
        }
    }

    obj.write()
}

fn dump_bytes(
    label: &str,
    bytes: &[u8],
    show_bytes: bool,
    dump_hex: bool,
    dump_bin: bool,
    dump_json: bool,
) {
    const LIMIT: usize = 256;
    let n = bytes.len().min(LIMIT);
    let head = &bytes[..n];

    if show_bytes {
        println!(
            "== BYTES ({}, {} bytes, head {} bytes) ==",
            label,
            bytes.len(),
            n
        );
        for (i, b) in head.iter().enumerate() {
            println!("{:04X}: {}", i, b);
        }
    }

    if dump_hex {
        println!("== HEX ({}, head {} bytes) ==", label, n);
        for (i, chunk) in head.chunks(16).enumerate() {
            print!("{:04X}: ", i * 16);
            for b in chunk {
                print!("{:02X} ", b);
            }
            println!();
        }
    }

    if dump_bin {
        println!("== BIN ({}, head {} bytes) ==", label, n);
        for (i, b) in head.iter().enumerate() {
            println!("{:04X}: {:08b}", i, b);
        }
    }

    if dump_json {
        println!("== JSON ({}) ==", label);
        println!("{{");
        println!("  \"len\": {},", bytes.len());
        print!("  \"head\": [");
        for (i, b) in head.iter().enumerate() {
            if i != 0 {
                print!(", ");
            }
            print!("{}", b);
        }
        println!("]");
        println!("}}");
    }
}

fn print_help() {
    println!("Usage:");
    println!("  whale asm --amd64 <input> -o <output.o>");
    println!();
    println!("Options:");
    println!("  --debug-whale   enable debug features");
    println!("  --ast           print parser AST (debug)");
    println!("  --token         print tokens (debug)");
    println!("  --bytes         print object bytes (debug)");
    println!("  --dump-hex      print object bytes as hex (debug)");
    println!("  --dump-bin      print object bytes as binary (debug)");
    println!("  --dump-json     print object bytes as json (debug)");
    println!("  --stats         show stats (debug)");
    println!("  --trace         trace logs");
}
