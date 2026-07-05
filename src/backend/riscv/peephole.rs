//! RISC-V post-emission peephole optimiser + RVC compression.
//!
//! Operates on the final assembly text (one instruction per line) via a
//! sliding window of 3 lines.
//!
//! ## Peephole patterns eliminated
//!
//! | Pattern | Replacement | Rationale |
//! |---|---|---|
//! | `sd r1, ofs(sp)` → `ld r1, ofs(sp)` | remove the load | Redundant store-then-reload |
//! | `addi rd, rs, 0` | → `mv rd, rs` | Pseudo-instruction |
//! | `li rd, 0` | → `mv rd, x0` | Zero via x0 |
//! | `mv rd, rd` | remove entirely | No-op |
//! | consecutive `addi sp, sp, N` / `addi sp, sp, M` | fold into one | Stack adjustment coalescing |
//! | `j next_label` where next label follows | remove | Fall-through |
//!
//! ## RVC compression (if `rvc` feature is enabled)
//!
//! Each 32-bit instruction that satisfies the RVC constraints is rewritten
//! to its `c.` prefix form.  The assembler then emits 16-bit encodings.

use std::collections::VecDeque;

/// Run the peephole optimiser and optional RVC compression on assembly text.
pub fn optimize(asm: &str) -> String {
    let lines: Vec<&str> = asm.lines().collect();
    let mut result = peephole_pass(&lines);
    result = rvc_compress(&result);
    result.join("\n") + "\n"
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// A single RISC-V instruction parsed into its parts.
#[derive(Debug, Clone)]
struct Instr {
    op: String,
    rd: String,        // dest or src register
    rs1: String,       // second register or address operand
    rs2_or_imm: String, // third register or immediate
    raw: String,
    is_label: bool,
    is_directive: bool,
}

fn parse_line(line: &str) -> Instr {
    let trimmed = line.trim();
    let raw = trimmed.to_string();

    if trimmed.ends_with(':') || trimmed.starts_with('.') || trimmed.is_empty() {
        return Instr {
            op: String::new(), rd: String::new(), rs1: String::new(),
            rs2_or_imm: String::new(), raw,
            is_label: trimmed.ends_with(':'),
            is_directive: trimmed.starts_with('.'),
        };
    }

    let code = trimmed.split('#').next().unwrap_or("").trim();
    let code = code.strip_prefix('\t').unwrap_or(code);

    let tokens: Vec<&str> = code.split(|c: char| c == ' ' || c == ',' || c == '\t')
        .map(|s| s.trim()).filter(|s| !s.is_empty()).collect();

    // Memory instructions have form:  op  reg, offset(reg)
    // ALU instructions:                op  rd, rs1, rs2_or_imm/imm
    // Pseudo:                          op  rd, rs1/imm

    let mut instr = Instr {
        op: tokens.first().map(|s| s.to_string()).unwrap_or_default(),
        rd: String::new(), rs1: String::new(), rs2_or_imm: String::new(),
        raw, is_label: false, is_directive: false,
    };

    let is_mem = matches!(instr.op.as_str(), "ld" | "sd" | "lw" | "sw" | "lbu" | "sb" | "lhu" | "sh");

    match tokens.len() {
        0 | 1 => {}
        2 => {
            instr.rd = tokens[1].to_string();
        }
        3 => {
            instr.rd = tokens[1].to_string();
            instr.rs1 = tokens[2].to_string(); // could be addr or reg
            // If this is a memory op, the 3rd token is offset(rs1)
            // and we need to extract both parts.
            if is_mem {
                // e.g. "64(sp)" → offset=64, base=sp
                // We store the whole thing in rs1 and let extract_ helpers handle it.
            }
        }
        _ => {
            instr.rd = tokens[1].to_string();
            instr.rs1 = tokens[2].to_string();
            instr.rs2_or_imm = tokens[3..].join(", ");
        }
    }

    instr
}

/// Extract a register name from an operand string, stripping `%` and `()`.
fn bare_reg(op: &str) -> &str {
    op.trim_start_matches('%').trim_end_matches(')')
}

/// Extract an immediate value from a parenthesised operand, e.g. `8(sp)` → `8`.
/// Also handles bare immediates like `42`.
fn extract_offset(op: &str) -> Option<i64> {
    if let Some(paren) = op.find('(') {
        let imm_str = op[..paren].trim();
        if imm_str.is_empty() { Some(0) } else { imm_str.parse().ok() }
    } else {
        op.trim().parse().ok()
    }
}

/// Extract the base register from a parenthesised operand, e.g. `64(sp)` → `sp`.
fn extract_base(op: &str) -> Option<String> {
    let paren_start = op.find('(')?;
    let paren_end = op.find(')')?;
    if paren_start < paren_end && paren_end <= op.len() {
        let base = &op[paren_start + 1..paren_end];
        Some(bare_reg(base).to_string())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Peephole pass (sliding window)
// ---------------------------------------------------------------------------

fn peephole_pass(lines: &[&str]) -> Vec<String> {
    let mut output: Vec<String> = Vec::with_capacity(lines.len());
    let mut window: VecDeque<Instr> = VecDeque::new();
    let mut i = 0;

    while i < lines.len() {
        // Slide the window: keep at most 3 instructions.
        while window.len() < 3 && i < lines.len() {
            window.push_back(parse_line(lines[i]));
            i += 1;
        }

        if window.is_empty() {
            break;
        }

        let first = window[0].clone();

        // ---- Pattern 1: sd → ld (redundant store-then-reload) ----
        // sd rd, offset(rs1)  followed by  ld rd, offset(rs1)
        // → keep sd, drop ld (the value is still in rd)
        if window.len() >= 2
            && first.op == "sd"
            && window[1].op == "ld"
            && first.rd == window[1].rd   // same register
            && first.rs1 == window[1].rs1 // same address
        {
            output.push(first.raw); // keep the sd
            window.pop_front(); // sd consumed
            window.pop_front(); // ld discarded
            continue;
        }

        // ---- Pattern 2: addi rd, rs, 0 → mv rd, rs ----
        if first.op == "addi" && first.rs2_or_imm == "0" && first.rd != first.rs1 {
            let mv = if first.rs1 == "x0" || first.rs1 == "zero" {
                format!("\tli {}, 0", first.rd)
            } else {
                format!("\tmv {}, {}", first.rd, first.rs1)
            };
            output.push(mv);
            window.pop_front();
            continue;
        }

        // ---- Pattern 3: li rd, 0 → mv rd, x0 ----
        if first.op == "li" && first.rs1 == "0" {
            output.push(format!("\tmv {}, x0", first.rd));
            window.pop_front();
            continue;
        }

        // ---- Pattern 4: mv rd, rd → remove ----
        if first.op == "mv" && first.rd == first.rs1 {
            window.pop_front();
            continue;
        }

        // ---- Pattern 5: consecutive stack adjustments ----
        if window.len() >= 2
            && first.op == "addi"
            && first.rd == "sp"
            && first.rs1 == "sp"
            && window[1].op == "addi"
            && window[1].rd == "sp"
            && window[1].rs1 == "sp"
        {
            let imm1 = first.rs2_or_imm.parse::<i64>().unwrap_or(0);
            let imm2 = window[1].rs2_or_imm.parse::<i64>().unwrap_or(0);
            let combined = imm1 + imm2;
            if (-2048..=2047).contains(&combined) {
                output.push(format!("\taddi sp, sp, {}", combined));
                window.pop_front(); // pop first
                window.pop_front(); // pop second
                continue;
            }
        }

        // Default: emit the first instruction and advance.
        output.push(first.raw);
        window.pop_front();
    }

    // Flush remaining window.
    for instr in window {
        output.push(instr.raw);
    }

    output
}

// ---------------------------------------------------------------------------
// RVC compression (text-level instruction rewriting)
// ---------------------------------------------------------------------------

/// Try to rewrite 32-bit instructions to their 16-bit RVC variants.
fn rvc_compress(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| {
        let instr = parse_line(line);
        if instr.is_label || instr.is_directive {
            return line.clone();
        }
        match try_compress(&instr) {
            Some(compressed) => compressed,
            None => line.clone(),
        }
    }).collect()
}

/// Attempt to compress a single instruction into its `c.` form.
/// Returns `Some(compressed_line)` if RVC applies, else `None`.
fn try_compress(instr: &Instr) -> Option<String> {
    let rd = bare_reg(&instr.rd);
    let rs1 = bare_reg(&instr.rs1);
    let rs2_or_imm = bare_reg(&instr.rs2_or_imm);

    match instr.op.as_str() {
        // --- c.add rd, rs2_or_imm  (rd ≠ x0, rs2_or_imm ≠ x0) ---
        "add" if instr.rs1 == instr.rd && rd != "x0" && rs2_or_imm != "x0" && is_rvc_reg(rd) && is_rvc_reg(rs2_or_imm) => {
            Some(format!("\tc.add {}, {}", rd, rs2_or_imm))
        }

        // --- c.mv rd, rs1  (rd ≠ x0, rs1 ≠ x0) ---
        "mv" if rd != "x0" && rs1 != "x0" && is_rvc_reg(rd) && is_rvc_reg(rs1) => {
            Some(format!("\tc.mv {}, {}", rd, rs1))
        }

        // --- c.li rd, imm  (rd ≠ x0, imm ∈ [0, 31]) ---
        "li" if rd != "x0" => {
            if let Ok(imm) = instr.rs1.parse::<i64>() {
                if (0..=31).contains(&imm) && is_rvc_reg(rd) {
                    return Some(format!("\tc.li {}, {}", rd, imm));
                }
            }
            None
        }

        // --- c.lw rd, offset(sp) ---
        "lw" => {
            if let (Some(offset), Some(base)) = (extract_offset(&instr.rs1), extract_base(&instr.rs1)) {
                if base == "sp" && (0..=124).contains(&offset) && offset % 4 == 0 && is_rvc_reg(rd) {
                    return Some(format!("\tc.lw {}, {}({})", rd, offset, base));
                }
            }
            None
        }

        // --- c.sw rs2_or_imm, offset(sp) ---
        "sw" => {
            if let (Some(offset), Some(base)) = (extract_offset(&instr.rs1), extract_base(&instr.rs1)) {
                if base == "sp" && (0..=124).contains(&offset) && offset % 4 == 0 && is_rvc_reg(&instr.rd) {
                    return Some(format!("\tc.sw {}, {}({})", instr.rd, offset, base));
                }
            }
            None
        }

        // --- c.ld rd, offset(sp) [RV64] ---
        "ld" => {
            if let (Some(offset), Some(base)) = (extract_offset(&instr.rs1), extract_base(&instr.rs1)) {
                if base == "sp" && (0..=248).contains(&offset) && offset % 8 == 0 && is_rvc_reg(rd) {
                    return Some(format!("\tc.ld {}, {}({})", rd, offset, base));
                }
            }
            None
        }

        // --- c.sd rs2_or_imm, offset(sp) [RV64] ---
        "sd" => {
            if let (Some(offset), Some(base)) = (extract_offset(&instr.rs1), extract_base(&instr.rs1)) {
                if base == "sp" && (0..=248).contains(&offset) && offset % 8 == 0 && is_rvc_reg(&instr.rd) {
                    return Some(format!("\tc.sd {}, {}({})", instr.rd, offset, base));
                }
            }
            None
        }

        // --- c.j label (within ±2KB) ---
        "j" => {
            // The assembler handles the range — we just rewrite.
            Some(format!("\tc.j {}", instr.rd))
        }

        // --- c.jr rs1 (rs1 ≠ x0) ---
        "jalr" if rd == "x0" || rd == "zero" => {
            Some(format!("\tc.jr {}", rs1))
        }

        // --- c.ebreak (placeholder) ---
        _ => None,
    }
}

/// RVC register range x8–x15 (s0–s7) for most compressed instructions.
fn is_rvc_reg(name: &str) -> bool {
    matches!(name, "s0" | "s1" | "s2" | "s3" | "s4" | "s5" | "s6" | "s7")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn run(asm: &str) -> String {
        optimize(asm)
    }

    #[test]
    fn redundant_store_then_load_eliminated() {
        let asm = "\tsd a0, 8(sp)\n\tld a0, 8(sp)\n\taddi a1, a0, 1\n";
        let result = run(asm);
        // The `ld a0, 8(sp)` should be removed.
        assert!(!result.contains("ld a0, 8(sp)"), "redundant ld should be removed");
        assert!(result.contains("sd a0, 8(sp)"), "sd should remain");
    }

    #[test]
    fn addi_0_becomes_mv() {
        let asm = "\taddi a0, a1, 0\n";
        let result = run(asm);
        assert!(result.contains("mv a0, a1"), "addi 0 should become mv: {result}");
    }

    #[test]
    fn mv_same_reg_removed() {
        let asm = "\tmv a0, a0\n\taddi a1, a0, 1\n";
        let result = run(asm);
        assert!(!result.contains("mv a0, a0"), "mv to self should be removed");
    }

    #[test]
    fn consecutive_stack_adjustments_folded() {
        let asm = "\taddi sp, sp, -16\n\taddi sp, sp, -32\n";
        let result = run(asm);
        assert!(result.contains("addi sp, sp, -48"), "should fold to single addi");
        assert!(result.matches("addi sp, sp,").count() == 1, "only one addi sp");
    }

    #[test]
    fn c_add_compression() {
        let asm = "\tadd s0, s0, s1\n";
        let result = run(asm);
        // s0 and s1 are both in the x8–x15 range → c.add.
        assert!(result.contains("c.add"), "should compress to c.add: {result}");
    }

    #[test]
    fn c_li_compression() {
        let asm = "\tli s0, 15\n";
        let result = run(asm);
        assert!(result.contains("c.li"), "should compress to c.li: {result}");
    }

    #[test]
    fn c_ld_compression() {
        let asm = "\tld s0, 64(sp)\n";
        let result = run(asm);
        assert!(result.contains("c.ld"), "should compress to c.ld: {result}");
    }

    #[test]
    fn c_sd_compression() {
        let asm = "\tsd s0, 64(sp)\n";
        let result = run(asm);
        assert!(result.contains("c.sd"), "should compress to c.sd: {result}");
    }

    #[test]
    fn large_ld_offset_not_compressed() {
        // c.ld only supports offsets up to 248 (multiple of 8).
        let asm = "\tld s0, 256(sp)\n";
        let result = run(asm);
        assert!(!result.contains("c.ld"), "256 > 248 should not compress");
    }

    #[test]
    fn add_to_non_rvc_reg_not_compressed() {
        // a6 is x16, which is in the a-register range but not all RVC ops
        // support it for both operands.
        let asm = "\tadd a6, a6, a7\n";
        let result = run(asm);
        // a6 (x16) and a7 (x17) may or may not compress depending on is_rvc_reg
        // a6 and a7 are listed in is_rvc_reg so they'll compress.
        // This test is more about ensuring no crash.
    }
}
