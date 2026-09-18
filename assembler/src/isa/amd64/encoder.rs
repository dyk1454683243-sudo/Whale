use std::collections::{HashMap, HashSet};

use crate::assembler::{AsmSection, AsmSymbol, AssemblerOutput, RelocKind, Relocation};
use crate::ast::*;
use crate::error::AsmError;
use crate::isa::amd64::encoding::{encode_address, DispKind, EncodedAddress, ModRM, REX};
use crate::isa::amd64::tables::*;

pub fn encode(ast: &AST) -> Result<AssemblerOutput, AsmError> {
    const MAX_RELAX_ITERATIONS: usize = 8;

    let mut prev_label_locs: Option<HashMap<String, (usize, usize)>> = None;
    let mut last_output: Option<AssemblerOutput> = None;

    for _ in 0..MAX_RELAX_ITERATIONS {
        let (out, label_locs) = encode_once(ast, prev_label_locs.as_ref())?;
        if let Some(prev) = &prev_label_locs {
            if *prev == label_locs {
                return Ok(out);
            }
        }
        prev_label_locs = Some(label_locs);
        last_output = Some(out);
    }

    last_output.ok_or_else(|| AsmError::EncodeError("internal encoder error".into()))
}

fn encode_once(
    ast: &AST,
    jump_hint_locs: Option<&HashMap<String, (usize, usize)>>,
) -> Result<(AssemblerOutput, HashMap<String, (usize, usize)>), AsmError> {
    let mut sections = Vec::new();
    let mut symbols = Vec::new();

    let mut consts = HashMap::<String, i64>::new();
    let mut defined_labels = HashSet::<String>::new();
    let mut label_locs = HashMap::<String, (usize, usize)>::new();
    let mut jump_known_locs = jump_hint_locs.cloned().unwrap_or_default();
    let mut extern_symbols = HashSet::<String>::new();
    let mut global_symbols = HashSet::<String>::new();

    sections.push(AsmSection {
        name: ".text".to_string(),
        data: Vec::new(),
        relocs: Vec::new(),
    });

    let mut current_section_idx = 0usize;
    let mut current_nonlocal_label: Option<String> = None;

    for node in &ast.items {
        match node {
            ASTNode::Section(name) => {
                if let Some(idx) = sections.iter().position(|s| s.name == *name) {
                    current_section_idx = idx;
                } else {
                    sections.push(AsmSection {
                        name: name.clone(),
                        data: Vec::new(),
                        relocs: Vec::new(),
                    });
                    current_section_idx = sections.len() - 1;
                }
            }
            ASTNode::Global(name) => {
                let resolved = resolve_symbol_name(name, &current_nonlocal_label)?;
                global_symbols.insert(resolved);
            }
            ASTNode::Extern(name) => {
                let resolved = resolve_symbol_name(name, &current_nonlocal_label)?;
                if defined_labels.contains(&resolved) {
                    return Err(AsmError::SymbolError(format!(
                        "Extern symbol '{}' was already defined in this file",
                        resolved
                    )));
                }
                extern_symbols.insert(resolved.clone());
                symbols.push(AsmSymbol {
                    name: resolved,
                    section_index: None,
                    offset: 0,
                    is_global: true,
                });
            }
            ASTNode::Const { name, expr } => {
                let resolved_name = resolve_symbol_name(name, &current_nonlocal_label)?;
                if defined_labels.contains(&resolved_name) || extern_symbols.contains(&resolved_name) {
                    return Err(AsmError::SymbolError(format!(
                        "Constant '{}' conflicts with an existing symbol",
                        resolved_name
                    )));
                }
                if consts.contains_key(&resolved_name) {
                    return Err(AsmError::SymbolError(format!(
                        "Duplicate constant definition '{}'",
                        resolved_name
                    )));
                }

                let scoped_expr = resolve_expr_symbols(expr, &current_nonlocal_label)?;
                let value = match eval_expr(&scoped_expr, &consts) {
                    EvaluatedExpr::Number(v) => v,
                    EvaluatedExpr::Symbol { name, .. } => {
                        return Err(AsmError::SymbolError(format!(
                            "equ requires a constant expression; unresolved symbol '{}'",
                            name
                        )))
                    }
                };
                consts.insert(resolved_name, value);
            }
            ASTNode::Label(name) => {
                let resolved = resolve_symbol_name(name, &current_nonlocal_label)?;
                if !name.starts_with('.') {
                    current_nonlocal_label = Some(resolved.clone());
                }
                if extern_symbols.contains(&resolved) {
                    return Err(AsmError::SymbolError(format!(
                        "Label '{}' conflicts with extern declaration",
                        resolved
                    )));
                }
                if consts.contains_key(&resolved) {
                    return Err(AsmError::SymbolError(format!(
                        "Label '{}' conflicts with constant definition",
                        resolved
                    )));
                }
                if !defined_labels.insert(resolved.clone()) {
                    return Err(AsmError::SymbolError(format!(
                        "Duplicate label definition '{}'",
                        resolved
                    )));
                }
                let offset = sections[current_section_idx].data.len();
                label_locs.insert(resolved.clone(), (current_section_idx, offset));
                jump_known_locs.insert(resolved.clone(), (current_section_idx, offset));
                symbols.push(AsmSymbol {
                    name: resolved.clone(),
                    section_index: Some(current_section_idx),
                    offset,
                    is_global: global_symbols.contains(&resolved),
                });
            }
            ASTNode::Instruction(ins) => {
                let scoped = resolve_instruction_symbols(ins, &current_nonlocal_label)?;
                let inst = resolve_instruction_consts(&scoped, &consts);
                let sec = &mut sections[current_section_idx];
                let cur_off = sec.data.len();
                encode_instruction(
                    &inst,
                    &mut sec.data,
                    &mut sec.relocs,
                    current_section_idx,
                    cur_off,
                    &jump_known_locs,
                )?;
            }
            ASTNode::Directive(dir) => {
                let scoped = resolve_directive_symbols(dir, &current_nonlocal_label)?;
                let sec = &mut sections[current_section_idx];
                encode_directive(&scoped, &mut sec.data, &mut sec.relocs, &consts)?;
            }
        }
    }

    for sym in &mut symbols {
        if global_symbols.contains(&sym.name) {
            sym.is_global = true;
        }
    }

    for sec in &sections {
        for reloc in &sec.relocs {
            if !defined_labels.contains(&reloc.symbol)
                && !extern_symbols.contains(&reloc.symbol)
                && !consts.contains_key(&reloc.symbol)
            {
                return Err(AsmError::SymbolError(format!(
                    "Undefined symbol '{}' (define it, declare with extern, or define with equ)",
                    reloc.symbol
                )));
            }
        }
    }

    Ok((AssemblerOutput { sections, symbols }, label_locs))
}

#[derive(Clone)]
struct RegInfo {
    code: u8,
    width: u8,
    high8: bool,
}

#[derive(Clone)]
enum EvaluatedExpr {
    Number(i64),
    Symbol { name: String, addend: i64 },
}

fn resolve_symbol_name(raw: &str, current_nonlocal_label: &Option<String>) -> Result<String, AsmError> {
    if raw.starts_with('.') {
        if let Some(base) = current_nonlocal_label {
            return Ok(format!("{}{}", base, raw));
        }
        return Err(AsmError::SymbolError(format!(
            "Local label '{}' has no parent non-local label in scope",
            raw
        )));
    }
    Ok(raw.to_string())
}

fn resolve_expr_symbols(expr: &ExprValue, current_nonlocal_label: &Option<String>) -> Result<ExprValue, AsmError> {
    match expr {
        ExprValue::Number(n) => Ok(ExprValue::Number(*n)),
        ExprValue::Symbol { name, addend } => Ok(ExprValue::Symbol {
            name: resolve_symbol_name(name, current_nonlocal_label)?,
            addend: *addend,
        }),
    }
}

fn eval_expr(expr: &ExprValue, consts: &HashMap<String, i64>) -> EvaluatedExpr {
    match expr {
        ExprValue::Number(n) => EvaluatedExpr::Number(*n),
        ExprValue::Symbol { name, addend } => {
            if let Some(c) = consts.get(name) {
                EvaluatedExpr::Number(c + addend)
            } else {
                EvaluatedExpr::Symbol {
                    name: name.clone(),
                    addend: *addend,
                }
            }
        }
    }
}

fn resolve_instruction_symbols(
    ins: &Instruction,
    current_nonlocal_label: &Option<String>,
) -> Result<Instruction, AsmError> {
    let mut resolved_ops = Vec::with_capacity(ins.operands.len());
    for op in &ins.operands {
        match op {
            Operand::Label(sym) => resolved_ops.push(Operand::Label(resolve_symbol_name(
                sym,
                current_nonlocal_label,
            )?)),
            Operand::SymbolExpr { name, addend } => {
                resolved_ops.push(Operand::SymbolExpr {
                    name: resolve_symbol_name(name, current_nonlocal_label)?,
                    addend: *addend,
                });
            }
            Operand::Memory(mem) => {
                let mut mem = mem.clone();
                if let Some(sym) = &mem.symbol {
                    mem.symbol = Some(resolve_symbol_name(sym, current_nonlocal_label)?);
                }
                resolved_ops.push(Operand::Memory(mem));
            }
            _ => resolved_ops.push(op.clone()),
        }
    }
    Ok(Instruction {
        mnemonic: ins.mnemonic.clone(),
        operands: resolved_ops,
    })
}

fn resolve_instruction_consts(ins: &Instruction, consts: &HashMap<String, i64>) -> Instruction {
    let mut resolved_ops = Vec::with_capacity(ins.operands.len());
    for op in &ins.operands {
        match op {
            Operand::Label(name) => {
                if let Some(v) = consts.get(name) {
                    resolved_ops.push(Operand::Immediate(*v));
                } else {
                    resolved_ops.push(op.clone());
                }
            }
            Operand::SymbolExpr { name, addend } => {
                if let Some(v) = consts.get(name) {
                    resolved_ops.push(Operand::Immediate(*v + addend));
                } else {
                    resolved_ops.push(op.clone());
                }
            }
            Operand::Memory(mem) => {
                let mut mem = mem.clone();
                if let Some(sym) = &mem.symbol {
                    if let Some(v) = consts.get(sym) {
                        mem.disp += *v;
                        mem.symbol = None;
                    }
                }
                resolved_ops.push(Operand::Memory(mem));
            }
            _ => resolved_ops.push(op.clone()),
        }
    }
    Instruction {
        mnemonic: ins.mnemonic.clone(),
        operands: resolved_ops,
    }
}

fn resolve_directive_symbols(
    dir: &Directive,
    current_nonlocal_label: &Option<String>,
) -> Result<Directive, AsmError> {
    let mut out_values = Vec::with_capacity(dir.values.len());
    for v in &dir.values {
        match v {
            DirectiveValue::StringLiteral(s) => out_values.push(DirectiveValue::StringLiteral(s.clone())),
            DirectiveValue::Expr(expr) => out_values.push(DirectiveValue::Expr(resolve_expr_symbols(
                expr,
                current_nonlocal_label,
            )?)),
        }
    }
    Ok(Directive {
        name: dir.name.clone(),
        values: out_values,
    })
}

fn lookup_reg(name: &str) -> Option<RegInfo> {
    if let Some((_, code)) = REGISTERS_64.iter().find(|(n, _)| *n == name) {
        return Some(RegInfo {
            code: *code,
            width: 64,
            high8: false,
        });
    }
    if let Some((_, code)) = REGISTERS_32.iter().find(|(n, _)| *n == name) {
        return Some(RegInfo {
            code: *code,
            width: 32,
            high8: false,
        });
    }
    if let Some((_, code)) = REGISTERS_16.iter().find(|(n, _)| *n == name) {
        return Some(RegInfo {
            code: *code,
            width: 16,
            high8: false,
        });
    }
    if let Some((reg, code)) = REGISTERS_8.iter().find(|(n, _)| *n == name) {
        return Some(RegInfo {
            code: *code,
            width: 8,
            high8: matches!(*reg, "ah" | "bh" | "ch" | "dh"),
        });
    }
    None
}

fn emit_operand_size_prefix(bytes: &mut Vec<u8>, width: u8) {
    if width == 16 {
        bytes.push(0x66);
    }
}

fn emit_rex(
    bytes: &mut Vec<u8>,
    width: u8,
    rex_r: bool,
    rex_x: bool,
    rex_b: bool,
    any_high8: bool,
) -> Result<(), AsmError> {
    let mut rex = REX::new();
    rex.w = width == 64;
    rex.r = rex_r;
    rex.x = rex_x;
    rex.b = rex_b;

    let needed = rex.w || rex.r || rex.x || rex.b;
    if any_high8 && needed {
        return Err(AsmError::EncodeError(
            "AH/CH/DH/BH cannot be encoded with a REX prefix".into(),
        ));
    }
    if needed {
        bytes.push(rex.encode());
    }
    Ok(())
}

fn emit_disp(bytes: &mut Vec<u8>, disp: Option<DispKind>) {
    if let Some(d) = disp {
        match d {
            DispKind::Disp8(v) => bytes.push(v as u8),
            DispKind::Disp32(v) => bytes.extend_from_slice(&v.to_le_bytes()),
        }
    }
}

fn emit_addr_tail(bytes: &mut Vec<u8>, addr: &EncodedAddress) {
    if let Some((scale, index, base)) = addr.sib {
        bytes.push((scale << 6) | (index << 3) | base);
    }
    emit_disp(bytes, addr.disp.clone());
}

fn emit_reg_reg(bytes: &mut Vec<u8>, opcode: u8, dst: &RegInfo, src: &RegInfo) -> Result<(), AsmError> {
    emit_operand_size_prefix(bytes, dst.width);
    emit_rex(
        bytes,
        dst.width,
        src.code >= 8,
        false,
        dst.code >= 8,
        dst.high8 || src.high8,
    )?;
    bytes.push(opcode);
    bytes.push(ModRM::new(0b11, src.code, dst.code).encode());
    Ok(())
}

fn emit_reg_mem(
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
    opcode: u8,
    reg: &RegInfo,
    mem: &MemoryOperand,
) -> Result<(), AsmError> {
    emit_operand_size_prefix(bytes, reg.width);

    if let Some(sym) = &mem.symbol {
        if mem.base.is_some() || mem.index.is_some() {
            return Err(AsmError::EncodeError(
                "symbol + register memory form is not implemented yet".into(),
            ));
        }

        emit_rex(bytes, reg.width, reg.code >= 8, false, false, reg.high8)?;
        bytes.push(opcode);
        bytes.push(ModRM::new(0, reg.code, 5).encode()); // [rip + disp32]

        relocs.push(Relocation {
            offset: bytes.len(),
            symbol: sym.clone(),
            kind: RelocKind::Relative32,
            addend: mem.disp - 4,
        });
        bytes.extend_from_slice(&0i32.to_le_bytes());
        return Ok(());
    }

    let addr = encode_address(mem, 64)?;
    emit_rex(
        bytes,
        reg.width,
        reg.code >= 8,
        addr.rex_x,
        addr.rex_b,
        reg.high8,
    )?;
    bytes.push(opcode);
    bytes.push(ModRM::new(addr.mod_bits, reg.code, addr.rm_bits).encode());
    emit_addr_tail(bytes, &addr);
    Ok(())
}

fn emit_mem_reg(
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
    opcode: u8,
    mem: &MemoryOperand,
    reg: &RegInfo,
) -> Result<(), AsmError> {
    emit_operand_size_prefix(bytes, reg.width);

    if let Some(sym) = &mem.symbol {
        if mem.base.is_some() || mem.index.is_some() {
            return Err(AsmError::EncodeError(
                "symbol + register memory form is not implemented yet".into(),
            ));
        }

        emit_rex(bytes, reg.width, reg.code >= 8, false, false, reg.high8)?;
        bytes.push(opcode);
        bytes.push(ModRM::new(0, reg.code, 5).encode()); // [rip + disp32]

        relocs.push(Relocation {
            offset: bytes.len(),
            symbol: sym.clone(),
            kind: RelocKind::Relative32,
            addend: mem.disp - 4,
        });
        bytes.extend_from_slice(&0i32.to_le_bytes());
        return Ok(());
    }

    let addr = encode_address(mem, 64)?;
    emit_rex(
        bytes,
        reg.width,
        reg.code >= 8,
        addr.rex_x,
        addr.rex_b,
        reg.high8,
    )?;
    bytes.push(opcode);
    bytes.push(ModRM::new(addr.mod_bits, reg.code, addr.rm_bits).encode());
    emit_addr_tail(bytes, &addr);
    Ok(())
}

fn encode_instruction(
    ins: &Instruction,
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
    current_section: usize,
    current_offset: usize,
    label_locs: &HashMap<String, (usize, usize)>,
) -> Result<(), AsmError> {
    match ins.mnemonic.as_str() {
        "mov" => encode_mov(ins, bytes, relocs),
        "add" => encode_binop(ins, 0x00, 0x01, 0x02, 0x03, 0, bytes, relocs),
        "sub" => encode_binop(ins, 0x28, 0x29, 0x2A, 0x2B, 5, bytes, relocs),
        "and" => encode_binop(ins, 0x20, 0x21, 0x22, 0x23, 4, bytes, relocs),
        "or" => encode_binop(ins, 0x08, 0x09, 0x0A, 0x0B, 1, bytes, relocs),
        "xor" => encode_binop(ins, 0x30, 0x31, 0x32, 0x33, 6, bytes, relocs),
        "cmp" => encode_binop(ins, 0x38, 0x39, 0x3A, 0x3B, 7, bytes, relocs),
        "imul" => encode_imul(ins, bytes, relocs),
        "push" => encode_push_pop(ins, 0x50, bytes),
        "pop" => encode_push_pop(ins, 0x58, bytes),
        "jmp" => encode_jump(
            ins,
            JumpSpec {
                near_opcode: JumpOpcode::One(0xE9),
                short_opcode: Some(0xEB),
                reloc_kind: RelocKind::Relative32,
                reloc_addend: -4,
                near_len: 5,
            },
            bytes,
            relocs,
            current_section,
            current_offset,
            label_locs,
        ),
        "call" => encode_jump(
            ins,
            JumpSpec {
                near_opcode: JumpOpcode::One(0xE8),
                short_opcode: None,
                reloc_kind: RelocKind::Relative32,
                reloc_addend: -4,
                near_len: 5,
            },
            bytes,
            relocs,
            current_section,
            current_offset,
            label_locs,
        ),
        "je" => encode_jump(
            ins,
            JumpSpec {
                near_opcode: JumpOpcode::Two(0x0F, 0x84),
                short_opcode: Some(0x74),
                reloc_kind: RelocKind::Relative32,
                reloc_addend: -4,
                near_len: 6,
            },
            bytes,
            relocs,
            current_section,
            current_offset,
            label_locs,
        ),
        "loop" => encode_loop(ins, bytes, relocs, current_section, current_offset, label_locs),
        "ret" => {
            bytes.push(0xC3);
            Ok(())
        }
        "nop" => {
            bytes.push(0x90);
            Ok(())
        }
        "syscall" => {
            bytes.push(0x0F);
            bytes.push(0x05);
            Ok(())
        }
        "int3" => {
            bytes.push(0xCC);
            Ok(())
        }
        _ => Err(AsmError::EncodeError(format!(
            "Unknown mnemonic {}",
            ins.mnemonic
        ))),
    }
}

fn symbol_operand(op: &Operand) -> Option<(String, i64)> {
    match op {
        Operand::Label(name) => Some((name.clone(), 0)),
        Operand::SymbolExpr { name, addend } => Some((name.clone(), *addend)),
        _ => None,
    }
}

fn encode_mov(
    ins: &Instruction,
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
) -> Result<(), AsmError> {
    if ins.operands.len() != 2 {
        return Err(AsmError::EncodeError("mov expects 2 operands".into()));
    }
    let dst = &ins.operands[0];
    let src = &ins.operands[1];

    match (dst, src) {
        (Operand::Register(dst_name), Operand::Immediate(imm)) => {
            let reg = lookup_reg(dst_name).ok_or(AsmError::EncodeError("Invalid register".into()))?;
            match reg.width {
                8 => {
                    if !(-128..=255).contains(imm) {
                        return Err(AsmError::EncodeError("imm8 out of range".into()));
                    }
                    emit_rex(bytes, reg.width, false, false, reg.code >= 8, reg.high8)?;
                    bytes.push(0xB0 + (reg.code & 7));
                    bytes.push(*imm as u8);
                }
                16 => {
                    if !(-32768..=65535).contains(imm) {
                        return Err(AsmError::EncodeError("imm16 out of range".into()));
                    }
                    emit_operand_size_prefix(bytes, reg.width);
                    emit_rex(bytes, reg.width, false, false, reg.code >= 8, false)?;
                    bytes.push(0xB8 + (reg.code & 7));
                    bytes.extend_from_slice(&(*imm as i16).to_le_bytes());
                }
                32 => {
                    if !(*imm >= i32::MIN as i64 && *imm <= u32::MAX as i64) {
                        return Err(AsmError::EncodeError("imm32 out of range".into()));
                    }
                    emit_rex(bytes, reg.width, false, false, reg.code >= 8, false)?;
                    bytes.push(0xB8 + (reg.code & 7));
                    bytes.extend_from_slice(&(*imm as u32).to_le_bytes());
                }
                64 => {
                    emit_rex(bytes, reg.width, false, false, reg.code >= 8, false)?;
                    bytes.push(0xB8 + (reg.code & 7));
                    bytes.extend_from_slice(&imm.to_le_bytes());
                }
                _ => return Err(AsmError::EncodeError("Unsupported register width".into())),
            }
            Ok(())
        }
        (Operand::Register(dst_name), _) if symbol_operand(src).is_some() => {
            let (label, addend) = symbol_operand(src).unwrap();
            let reg = lookup_reg(dst_name).ok_or(AsmError::EncodeError("Invalid register".into()))?;
            match reg.width {
                64 => {
                    emit_rex(bytes, reg.width, false, false, reg.code >= 8, false)?;
                    bytes.push(0xB8 + (reg.code & 7));
                    relocs.push(Relocation {
                        offset: bytes.len(),
                        symbol: label,
                        kind: RelocKind::Absolute64,
                        addend,
                    });
                    bytes.extend_from_slice(&0i64.to_le_bytes());
                }
                32 => {
                    emit_rex(bytes, reg.width, false, false, reg.code >= 8, false)?;
                    bytes.push(0xB8 + (reg.code & 7));
                    relocs.push(Relocation {
                        offset: bytes.len(),
                        symbol: label,
                        kind: RelocKind::Absolute32,
                        addend,
                    });
                    bytes.extend_from_slice(&0u32.to_le_bytes());
                }
                _ => {
                    return Err(AsmError::EncodeError(
                        "Symbol move is supported only for r32/r64".into(),
                    ))
                }
            }
            Ok(())
        }
        (Operand::Register(dst_name), Operand::Register(src_name)) => {
            let dst_reg = lookup_reg(dst_name).ok_or(AsmError::EncodeError("Invalid dst register".into()))?;
            let src_reg = lookup_reg(src_name).ok_or(AsmError::EncodeError("Invalid src register".into()))?;
            if dst_reg.width != src_reg.width {
                return Err(AsmError::EncodeError("Register width mismatch".into()));
            }
            let opcode = if dst_reg.width == 8 { 0x88 } else { 0x89 };
            emit_reg_reg(bytes, opcode, &dst_reg, &src_reg)
        }
        (Operand::Register(dst_name), Operand::Memory(mem)) => {
            let reg = lookup_reg(dst_name).ok_or(AsmError::EncodeError("Invalid register".into()))?;
            let opcode = if reg.width == 8 { 0x8A } else { 0x8B };
            emit_reg_mem(bytes, relocs, opcode, &reg, mem)
        }
        (Operand::Memory(mem), Operand::Register(src_name)) => {
            let reg = lookup_reg(src_name).ok_or(AsmError::EncodeError("Invalid register".into()))?;
            let opcode = if reg.width == 8 { 0x88 } else { 0x89 };
            emit_mem_reg(bytes, relocs, opcode, mem, &reg)
        }
        _ => Err(AsmError::EncodeError("Unsupported mov form".into())),
    }
}

fn encode_binop(
    ins: &Instruction,
    opcode_rm_r_8: u8,
    opcode_rm_r: u8,
    opcode_r_rm_8: u8,
    opcode_r_rm: u8,
    imm_op_ext: u8,
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
) -> Result<(), AsmError> {
    if ins.operands.len() != 2 {
        return Err(AsmError::EncodeError(format!(
            "{} expects 2 operands",
            ins.mnemonic
        )));
    }
    let dst = &ins.operands[0];
    let src = &ins.operands[1];

    match (dst, src) {
        (Operand::Register(dst_name), Operand::Register(src_name)) => {
            let dst_reg = lookup_reg(dst_name).ok_or(AsmError::EncodeError("Invalid dst register".into()))?;
            let src_reg = lookup_reg(src_name).ok_or(AsmError::EncodeError("Invalid src register".into()))?;
            if dst_reg.width != src_reg.width {
                return Err(AsmError::EncodeError("Register width mismatch".into()));
            }
            let opcode = if dst_reg.width == 8 {
                opcode_rm_r_8
            } else {
                opcode_rm_r
            };
            emit_reg_reg(bytes, opcode, &dst_reg, &src_reg)
        }
        (Operand::Register(dst_name), Operand::Immediate(imm)) => {
            let dst_reg = lookup_reg(dst_name).ok_or(AsmError::EncodeError("Invalid dst register".into()))?;
            emit_operand_size_prefix(bytes, dst_reg.width);
            emit_rex(
                bytes,
                dst_reg.width,
                false,
                false,
                dst_reg.code >= 8,
                dst_reg.high8,
            )?;

            if dst_reg.width == 8 {
                if !(-128..=255).contains(imm) {
                    return Err(AsmError::EncodeError("imm8 out of range".into()));
                }
                bytes.push(0x80);
                bytes.push(ModRM::new(0b11, imm_op_ext, dst_reg.code).encode());
                bytes.push(*imm as u8);
                return Ok(());
            }

            if (-128..=127).contains(imm) {
                bytes.push(0x83);
                bytes.push(ModRM::new(0b11, imm_op_ext, dst_reg.code).encode());
                bytes.push(*imm as u8);
                return Ok(());
            }

            bytes.push(0x81);
            bytes.push(ModRM::new(0b11, imm_op_ext, dst_reg.code).encode());
            match dst_reg.width {
                16 => {
                    if !(-32768..=65535).contains(imm) {
                        return Err(AsmError::EncodeError("imm16 out of range".into()));
                    }
                    bytes.extend_from_slice(&(*imm as i16).to_le_bytes());
                }
                32 => {
                    if !(*imm >= i32::MIN as i64 && *imm <= u32::MAX as i64) {
                        return Err(AsmError::EncodeError("imm32 out of range".into()));
                    }
                    bytes.extend_from_slice(&(*imm as u32).to_le_bytes());
                }
                64 => {
                    if !(*imm >= i32::MIN as i64 && *imm <= i32::MAX as i64) {
                        return Err(AsmError::EncodeError(
                            "imm32-sign-extended out of range for r64".into(),
                        ));
                    }
                    bytes.extend_from_slice(&(*imm as i32).to_le_bytes());
                }
                _ => return Err(AsmError::EncodeError("Unsupported register width".into())),
            }
            Ok(())
        }
        (Operand::Register(_), _) if symbol_operand(src).is_some() => Err(AsmError::EncodeError(
            format!(
                "{} with symbolic immediate is not supported; use equ or mov reg, symbol",
                ins.mnemonic
            ),
        )),
        (Operand::Register(dst_name), Operand::Memory(mem)) => {
            let reg = lookup_reg(dst_name).ok_or(AsmError::EncodeError("Invalid register".into()))?;
            let opcode = if reg.width == 8 {
                opcode_r_rm_8
            } else {
                opcode_r_rm
            };
            emit_reg_mem(bytes, relocs, opcode, &reg, mem)
        }
        (Operand::Memory(mem), Operand::Register(src_name)) => {
            let reg = lookup_reg(src_name).ok_or(AsmError::EncodeError("Invalid register".into()))?;
            let opcode = if reg.width == 8 {
                opcode_rm_r_8
            } else {
                opcode_rm_r
            };
            emit_mem_reg(bytes, relocs, opcode, mem, &reg)
        }
        _ => Err(AsmError::EncodeError(format!(
            "Unsupported {} form",
            ins.mnemonic
        ))),
    }
}

fn encode_imul(
    ins: &Instruction,
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
) -> Result<(), AsmError> {
    if ins.operands.len() != 2 {
        return Err(AsmError::EncodeError("imul expects 2 operands".into()));
    }
    let dst = &ins.operands[0];
    let src = &ins.operands[1];

    let Operand::Register(dst_name) = dst else {
        return Err(AsmError::EncodeError(
            "imul form supported: imul reg, reg/mem".into(),
        ));
    };
    let dst_reg = lookup_reg(dst_name).ok_or(AsmError::EncodeError("Invalid dst register".into()))?;

    if dst_reg.width == 8 {
        return Err(AsmError::EncodeError(
            "imul reg, reg/mem is not supported for 8-bit operands".into(),
        ));
    }

    match src {
        Operand::Register(src_name) => {
            let src_reg = lookup_reg(src_name).ok_or(AsmError::EncodeError("Invalid src register".into()))?;
            if src_reg.width != dst_reg.width {
                return Err(AsmError::EncodeError("Register width mismatch".into()));
            }
            emit_operand_size_prefix(bytes, dst_reg.width);
            emit_rex(
                bytes,
                dst_reg.width,
                dst_reg.code >= 8,
                false,
                src_reg.code >= 8,
                false,
            )?;
            bytes.push(0x0F);
            bytes.push(0xAF);
            bytes.push(ModRM::new(0b11, dst_reg.code, src_reg.code).encode());
            Ok(())
        }
        Operand::Memory(mem) => {
            emit_operand_size_prefix(bytes, dst_reg.width);
            if let Some(sym) = &mem.symbol {
                if mem.base.is_some() || mem.index.is_some() {
                    return Err(AsmError::EncodeError(
                        "symbol + register memory form is not implemented yet".into(),
                    ));
                }
                emit_rex(bytes, dst_reg.width, dst_reg.code >= 8, false, false, false)?;
                bytes.push(0x0F);
                bytes.push(0xAF);
                bytes.push(ModRM::new(0, dst_reg.code, 5).encode());
                relocs.push(Relocation {
                    offset: bytes.len(),
                    symbol: sym.clone(),
                    kind: RelocKind::Relative32,
                    addend: mem.disp - 4,
                });
                bytes.extend_from_slice(&0i32.to_le_bytes());
                return Ok(());
            }

            let addr = encode_address(mem, 64)?;
            emit_rex(
                bytes,
                dst_reg.width,
                dst_reg.code >= 8,
                addr.rex_x,
                addr.rex_b,
                false,
            )?;
            bytes.push(0x0F);
            bytes.push(0xAF);
            bytes.push(ModRM::new(addr.mod_bits, dst_reg.code, addr.rm_bits).encode());
            emit_addr_tail(bytes, &addr);
            Ok(())
        }
        _ => Err(AsmError::EncodeError(
            "imul form supported: imul reg, reg/mem".into(),
        )),
    }
}

fn encode_push_pop(ins: &Instruction, base_opcode: u8, bytes: &mut Vec<u8>) -> Result<(), AsmError> {
    if ins.operands.len() != 1 {
        return Err(AsmError::EncodeError(format!(
            "{} expects 1 operand",
            ins.mnemonic
        )));
    }
    if let Operand::Register(name) = &ins.operands[0] {
        let reg = lookup_reg(name).ok_or(AsmError::EncodeError("Invalid register".into()))?;
        if reg.width == 16 {
            bytes.push(0x66);
        } else if reg.width != 64 {
            return Err(AsmError::EncodeError(
                "push/pop currently support only r16/r64 in amd64 mode".into(),
            ));
        }
        emit_rex(bytes, 0, false, false, reg.code >= 8, false)?;
        bytes.push(base_opcode + (reg.code & 7));
        Ok(())
    } else {
        Err(AsmError::EncodeError(format!(
            "{} only supports registers for now",
            ins.mnemonic
        )))
    }
}

enum JumpOpcode {
    One(u8),
    Two(u8, u8),
}

struct JumpSpec {
    near_opcode: JumpOpcode,
    short_opcode: Option<u8>,
    reloc_kind: RelocKind,
    reloc_addend: i64,
    near_len: usize,
}

fn emit_near_opcode(bytes: &mut Vec<u8>, opcode: &JumpOpcode) {
    match opcode {
        JumpOpcode::One(b) => bytes.push(*b),
        JumpOpcode::Two(a, b) => {
            bytes.push(*a);
            bytes.push(*b);
        }
    }
}

fn encode_jump(
    ins: &Instruction,
    spec: JumpSpec,
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
    current_section: usize,
    current_offset: usize,
    label_locs: &HashMap<String, (usize, usize)>,
) -> Result<(), AsmError> {
    if ins.operands.len() != 1 {
        return Err(AsmError::EncodeError(format!(
            "{} expects 1 operand",
            ins.mnemonic
        )));
    }
    let Some((label, addend)) = symbol_operand(&ins.operands[0]) else {
        return Err(AsmError::EncodeError(format!(
            "{} only supports labels for now",
            ins.mnemonic
        )));
    };
    if addend != 0 {
        return Err(AsmError::EncodeError(format!(
            "{} does not support label addends",
            ins.mnemonic
        )));
    }

    if let Some((target_sec, target_off)) = label_locs.get(&label) {
        if *target_sec == current_section {
            if let Some(short_op) = spec.short_opcode {
                let short_len = 2i64;
                let disp_short = *target_off as i64 - (current_offset as i64 + short_len);
                if (-128..=127).contains(&disp_short) {
                    bytes.push(short_op);
                    bytes.push(disp_short as i8 as u8);
                    return Ok(());
                }
            }

            let disp_near = *target_off as i64 - (current_offset as i64 + spec.near_len as i64);
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&disp_near) {
                return Err(AsmError::EncodeError(format!(
                    "{} target '{}' is out of 32-bit relative range",
                    ins.mnemonic, label
                )));
            }

            emit_near_opcode(bytes, &spec.near_opcode);
            bytes.extend_from_slice(&(disp_near as i32).to_le_bytes());
            return Ok(());
        }
    }

    emit_near_opcode(bytes, &spec.near_opcode);
    relocs.push(Relocation {
        offset: bytes.len(),
        symbol: label,
        kind: spec.reloc_kind,
        addend: spec.reloc_addend,
    });
    bytes.extend_from_slice(&0i32.to_le_bytes());
    Ok(())
}

fn encode_loop(
    ins: &Instruction,
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
    current_section: usize,
    current_offset: usize,
    label_locs: &HashMap<String, (usize, usize)>,
) -> Result<(), AsmError> {
    if ins.operands.len() != 1 {
        return Err(AsmError::EncodeError("loop expects 1 operand".into()));
    }
    let Some((label, addend)) = symbol_operand(&ins.operands[0]) else {
        return Err(AsmError::EncodeError("loop only supports labels for now".into()));
    };
    if addend != 0 {
        return Err(AsmError::EncodeError("loop does not support label addends".into()));
    }

    bytes.push(0xE2);

    if let Some((target_sec, target_off)) = label_locs.get(&label) {
        if *target_sec == current_section {
            let disp = *target_off as i64 - (current_offset as i64 + 2);
            if !(-128..=127).contains(&disp) {
                return Err(AsmError::EncodeError(format!(
                    "loop target '{}' is out of 8-bit range",
                    label
                )));
            }
            bytes.push(disp as i8 as u8);
            return Ok(());
        }
    }

    relocs.push(Relocation {
        offset: bytes.len(),
        symbol: label,
        kind: RelocKind::Relative8,
        addend: -1,
    });
    bytes.push(0);
    Ok(())
}

fn eval_directive_expr(
    expr: &ExprValue,
    consts: &HashMap<String, i64>,
) -> EvaluatedExpr {
    eval_expr(expr, consts)
}

fn encode_data_expr(
    eval: EvaluatedExpr,
    width: usize,
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
) -> Result<(), AsmError> {
    match eval {
        EvaluatedExpr::Number(n) => {
            match width {
                1 => {
                    if !(-128..=255).contains(&n) {
                        return Err(AsmError::EncodeError("db value out of range".into()));
                    }
                    bytes.push(n as u8);
                }
                2 => {
                    if !(-32768..=65535).contains(&n) {
                        return Err(AsmError::EncodeError("dw value out of range".into()));
                    }
                    bytes.extend_from_slice(&(n as i16).to_le_bytes());
                }
                4 => {
                    if !(i32::MIN as i64..=u32::MAX as i64).contains(&n) {
                        return Err(AsmError::EncodeError("dd value out of range".into()));
                    }
                    bytes.extend_from_slice(&(n as u32).to_le_bytes());
                }
                8 => {
                    bytes.extend_from_slice(&n.to_le_bytes());
                }
                _ => return Err(AsmError::EncodeError("Unsupported data width".into())),
            }
            Ok(())
        }
        EvaluatedExpr::Symbol { name, addend } => {
            match width {
                4 => {
                    relocs.push(Relocation {
                        offset: bytes.len(),
                        symbol: name,
                        kind: RelocKind::Absolute32,
                        addend,
                    });
                    bytes.extend_from_slice(&0u32.to_le_bytes());
                    Ok(())
                }
                8 => {
                    relocs.push(Relocation {
                        offset: bytes.len(),
                        symbol: name,
                        kind: RelocKind::Absolute64,
                        addend,
                    });
                    bytes.extend_from_slice(&0u64.to_le_bytes());
                    Ok(())
                }
                _ => Err(AsmError::EncodeError(
                    "Only dd/dq support symbolic expressions".into(),
                )),
            }
        }
    }
}

fn reserve_bytes(
    bytes: &mut Vec<u8>,
    unit: usize,
    values: &[DirectiveValue],
    consts: &HashMap<String, i64>,
) -> Result<(), AsmError> {
    if values.len() != 1 {
        return Err(AsmError::EncodeError("res* expects exactly one count operand".into()));
    }

    let count = match &values[0] {
        DirectiveValue::Expr(expr) => match eval_directive_expr(expr, consts) {
            EvaluatedExpr::Number(v) => v,
            EvaluatedExpr::Symbol { name, .. } => {
                return Err(AsmError::EncodeError(format!(
                    "res* count must be constant; unresolved symbol '{}'",
                    name
                )))
            }
        },
        DirectiveValue::StringLiteral(_) => {
            return Err(AsmError::EncodeError("res* count must be numeric expression".into()))
        }
    };

    if count < 0 {
        return Err(AsmError::EncodeError("res* count must be non-negative".into()));
    }
    let count = count as usize;
    let extra = count
        .checked_mul(unit)
        .ok_or_else(|| AsmError::EncodeError("res* size overflow".into()))?;
    let new_len = bytes
        .len()
        .checked_add(extra)
        .ok_or_else(|| AsmError::EncodeError("res* size overflow".into()))?;
    bytes.resize(new_len, 0);
    Ok(())
}

fn encode_directive(
    dir: &Directive,
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Relocation>,
    consts: &HashMap<String, i64>,
) -> Result<(), AsmError> {
    match dir.name.as_str() {
        "db" => {
            for v in &dir.values {
                match v {
                    DirectiveValue::StringLiteral(s) => bytes.extend_from_slice(s.as_bytes()),
                    DirectiveValue::Expr(expr) => {
                        let eval = eval_directive_expr(expr, consts);
                        encode_data_expr(eval, 1, bytes, relocs)?;
                    }
                }
            }
            Ok(())
        }
        "dw" => {
            for v in &dir.values {
                let DirectiveValue::Expr(expr) = v else {
                    return Err(AsmError::EncodeError("dw only supports numeric expressions".into()));
                };
                let eval = eval_directive_expr(expr, consts);
                encode_data_expr(eval, 2, bytes, relocs)?;
            }
            Ok(())
        }
        "dd" => {
            for v in &dir.values {
                let DirectiveValue::Expr(expr) = v else {
                    return Err(AsmError::EncodeError("dd only supports numeric expressions".into()));
                };
                let eval = eval_directive_expr(expr, consts);
                encode_data_expr(eval, 4, bytes, relocs)?;
            }
            Ok(())
        }
        "dq" => {
            for v in &dir.values {
                let DirectiveValue::Expr(expr) = v else {
                    return Err(AsmError::EncodeError("dq only supports numeric expressions".into()));
                };
                let eval = eval_directive_expr(expr, consts);
                encode_data_expr(eval, 8, bytes, relocs)?;
            }
            Ok(())
        }
        "resb" => reserve_bytes(bytes, 1, &dir.values, consts),
        "resw" => reserve_bytes(bytes, 2, &dir.values, consts),
        "resd" => reserve_bytes(bytes, 4, &dir.values, consts),
        "resq" => reserve_bytes(bytes, 8, &dir.values, consts),
        _ => Err(AsmError::EncodeError(format!(
            "Unknown directive {}",
            dir.name
        ))),
    }
}
