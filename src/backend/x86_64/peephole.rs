//! Assembly-level peephole optimization pass.
//!
//! This pass runs *after* `codegen.rs` has produced the final x86-64 AT&T
//! assembly text. Because the naive stack-only codegen (see
//! `register_allocator.rs`) reloads every operand from memory for nearly
//! every instruction, the emitted text is full of small, syntactically
//! detectable redundancies (self-moves, store-then-immediate-reload pairs,
//! no-op arithmetic, jumps that fall through to their own target label).
//!
//! Unlike `optimizer.rs` (which rewrites the IR before codegen), this pass
//! works purely on the generated `&str`, line by line, and never needs to
//! understand IR semantics -- it only needs to know the *textual* shape that
//! `codegen.rs` emits (`emit_instr` -> `"    {op}\t{args}\n"`, `emit_line` ->
//! raw lines such as labels and directives).

/// How a single emitted line is classified for peephole purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineKind {
    /// A label definition, e.g. `foo:`, `.Lfoo_entry:`, `.Lfoo_entry.branch_true:`.
    Label,
    /// An assembler directive that is not a label, e.g. `.section .rodata`,
    /// `.globl main`, `.string "..."`.
    Directive,
    /// A blank line (from `emit_line("")`).
    Blank,
    /// A plain instruction line produced by `emit_instr`.
    Instruction,
}

fn classify(line: &str) -> LineKind {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        LineKind::Blank
    } else if trimmed.ends_with(':') {
        LineKind::Label
    } else if trimmed.starts_with('.') {
        LineKind::Directive
    } else {
        LineKind::Instruction
    }
}

/// Extracts the mnemonic (first whitespace/tab-delimited token) of an
/// instruction line, e.g. `"    movq\t%rax, -8(%rbp)"` -> `"movq"`.
fn mnemonic_of(line: &str) -> String {
    let trimmed = line.trim();
    let end = trimmed
        .find(|c: char| c == '\t' || c == ' ')
        .unwrap_or(trimmed.len());
    trimmed[..end].to_string()
}

/// True for instructions that transfer or may transfer control: `call`,
/// `ret`, `leave`, and any jump (`jmp`, `jne`, `je`, `jl`, ...). A peephole
/// rewrite must never assume "falls through to the next line" across one of
/// these.
fn is_control_flow_instr(line: &str) -> bool {
    let m = mnemonic_of(line);
    m == "call" || m == "ret" || m == "leave" || m.starts_with('j')
}

/// True for a line that is a plain, non-control-flow instruction -- the only
/// kind of line eligible to participate in an adjacent-pair peephole match.
fn is_plain_instruction(line: &str) -> bool {
    classify(line) == LineKind::Instruction && !is_control_flow_instr(line)
}

/// Parses an instruction line into `(mnemonic, operands)`, splitting the
/// comma-separated operand list the way `emit_instr`'s `args.join(", ")`
/// produced it.
fn parse_instr(line: &str) -> Option<(String, Vec<String>)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.splitn(2, '\t');
    let mnemonic = parts.next()?.to_string();
    let rest = parts.next().unwrap_or("").trim();
    if rest.is_empty() {
        return Some((mnemonic, Vec::new()));
    }
    let operands = rest.split(',').map(|s| s.trim().to_string()).collect();
    Some((mnemonic, operands))
}

/// Attempts a single rewrite anchored at line index `i`. Returns `true` (and
/// mutates `lines`) if a pattern matched and a line was removed.
fn try_rewrite(lines: &mut Vec<String>, i: usize) -> bool {
    // --- Single-line patterns -------------------------------------------------
    if is_plain_instruction(&lines[i])
        && let Some((mnemonic, operands)) = parse_instr(&lines[i])
    {
        // Self-move: movq X, X
        if mnemonic == "movq" && operands.len() == 2 && operands[0] == operands[1] {
            lines.remove(i);
            return true;
        }
        // No-op arithmetic: addq $0, R / subq $0, R
        if (mnemonic == "addq" || mnemonic == "subq") && operands.len() == 2 && operands[0] == "$0" {
            lines.remove(i);
            return true;
        }
        // No-op arithmetic: imulq $1, R
        if mnemonic == "imulq" && operands.len() == 2 && operands[0] == "$1" {
            lines.remove(i);
            return true;
        }
    }

    if i + 1 >= lines.len() {
        return false;
    }

    // --- jmp-to-next-label: an instruction -> label boundary pattern ---------
    // This is intentionally allowed to look across an instruction/label
    // boundary: `jmp` is a terminator, and if the very next textual line is
    // its own target label, the jump is provably redundant no matter what
    // precedes it (control falls through to the same place either way).
    if classify(&lines[i]) == LineKind::Instruction
        && let Some((mnemonic, operands)) = parse_instr(&lines[i])
        && mnemonic == "jmp"
        && operands.len() == 1
        && classify(&lines[i + 1]) == LineKind::Label
    {
        let target = operands[0].trim();
        let label_text = lines[i + 1].trim().trim_end_matches(':');
        if target == label_text {
            lines.remove(i);
            return true;
        }
    }

    // --- Adjacent-pair patterns: both lines must be plain instructions --------
    if is_plain_instruction(&lines[i]) && is_plain_instruction(&lines[i + 1]) {
        if let (Some((m1, ops1)), Some((m2, ops2))) = (parse_instr(&lines[i]), parse_instr(&lines[i + 1])) {
            // Store-then-reload / reload-then-store-back:
            // movq A, B  followed by  movq B, A  ->  drop the second line.
            if m1 == "movq" && m2 == "movq" && ops1.len() == 2 && ops2.len() == 2 {
                let (a, b) = (&ops1[0], &ops1[1]);
                let (b2, a2) = (&ops2[0], &ops2[1]);
                if a == a2 && b == b2 && a != b {
                    lines.remove(i + 1);
                    return true;
                }
            }
        }
    }

    false
}

pub struct Peephole;

impl Peephole {
    /// Runs the peephole pass to a fixpoint over generated assembly text.
    pub fn optimize(asm: &str) -> String {
        let trailing_newline = asm.ends_with('\n');
        let mut lines: Vec<String> = asm.lines().map(|s| s.to_string()).collect();

        const MAX_ITERATIONS: usize = 1000;
        let mut pass = 0;
        let mut changed = true;
        while changed && pass < MAX_ITERATIONS {
            changed = false;
            pass += 1;
            let mut i = 0;
            while i < lines.len() {
                if try_rewrite(&mut lines, i) {
                    changed = true;
                    // A removal may expose a new adjacent pair with the
                    // previous line, so step back one and recheck.
                    if i > 0 {
                        i -= 1;
                    }
                    continue;
                }
                i += 1;
            }
        }

        let mut out = lines.join("\n");
        if trailing_newline {
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_self_move() {
        let asm = "    movq\t%rax, %rax\n    movq\t%rax, -8(%rbp)\n";
        let out = Peephole::optimize(asm);
        assert!(!out.contains("movq\t%rax, %rax"));
        assert!(out.contains("movq\t%rax, -8(%rbp)"));
    }

    #[test]
    fn removes_store_then_reload_pair() {
        let asm = "    movq\t%r10, -8(%rbp)\n    movq\t-8(%rbp), %r10\n    movq\t%r10, %rax\n";
        let out = Peephole::optimize(asm);
        // Second line (the reload) should be gone.
        assert_eq!(out.matches("movq\t-8(%rbp), %r10").count(), 0);
        // First store and the trailing instruction survive.
        assert!(out.contains("movq\t%r10, -8(%rbp)"));
        assert!(out.contains("movq\t%r10, %rax"));
    }

    #[test]
    fn removes_reload_then_store_back_pair() {
        // Symmetric case: read into a register, then immediately write
        // straight back to the same location -- same (A,B)->(B,A) shape.
        let asm = "    movq\t-16(%rbp), %r11\n    movq\t%r11, -16(%rbp)\n";
        let out = Peephole::optimize(asm);
        assert_eq!(out.lines().filter(|l| !l.trim().is_empty()).count(), 1);
        assert!(out.contains("movq\t-16(%rbp), %r11"));
    }

    #[test]
    fn removes_noop_add_sub_zero() {
        let asm = "    addq\t$0, %rax\n    subq\t$0, %r10\n    movq\t%rax, %r10\n";
        let out = Peephole::optimize(asm);
        assert!(!out.contains("addq\t$0"));
        assert!(!out.contains("subq\t$0"));
        assert!(out.contains("movq\t%rax, %r10"));
    }

    #[test]
    fn removes_noop_imul_one() {
        let asm = "    imulq\t$1, %rax\n    movq\t%rax, -8(%rbp)\n";
        let out = Peephole::optimize(asm);
        assert!(!out.contains("imulq\t$1"));
        assert!(out.contains("movq\t%rax, -8(%rbp)"));
    }

    #[test]
    fn removes_redundant_jump_to_next_label() {
        let asm = "    jmp\t.Lfoo_bar\n.Lfoo_bar:\n    ret\n";
        let out = Peephole::optimize(asm);
        assert!(!out.contains("jmp\t.Lfoo_bar"));
        assert!(out.contains(".Lfoo_bar:"));
        assert!(out.contains("ret"));
    }

    #[test]
    fn does_not_collapse_store_reload_across_a_label() {
        // Something else may jump straight to `.Lfoo:` and expect %rax to
        // already hold the value stored just before it -- the reload after
        // the label is NOT a redundant pair with the store before it.
        let asm = "    movq\t%rax, -8(%rbp)\n.Lfoo:\n    movq\t-8(%rbp), %rax\n";
        let out = Peephole::optimize(asm);
        assert!(out.contains("movq\t%rax, -8(%rbp)"));
        assert!(out.contains(".Lfoo:"));
        assert!(out.contains("movq\t-8(%rbp), %rax"));
    }

    #[test]
    fn does_not_collapse_pair_across_a_call() {
        let asm = "    movq\t%rax, -8(%rbp)\n    call\tfoo\n    movq\t-8(%rbp), %rax\n";
        let out = Peephole::optimize(asm);
        assert!(out.contains("movq\t%rax, -8(%rbp)"));
        assert!(out.contains("call\tfoo"));
        assert!(out.contains("movq\t-8(%rbp), %rax"));
    }

    #[test]
    fn leaves_directives_and_labels_untouched() {
        let asm = ".section .rodata\n.LC_str_0:\n    .string \"hi\"\n.text\nfoo:\n    pushq\t%rbp\n";
        let out = Peephole::optimize(asm);
        assert_eq!(out, asm);
    }

    #[test]
    fn is_idempotent() {
        let asm = "    movq\t%rax, %rax\n    movq\t%r10, -8(%rbp)\n    movq\t-8(%rbp), %r10\n    addq\t$0, %rax\n    jmp\t.Lend\n.Lend:\n    ret\n";
        let once = Peephole::optimize(asm);
        let twice = Peephole::optimize(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn preserves_trailing_newline_behavior() {
        let with_newline = "    movq\t%rax, %rax\n";
        let out = Peephole::optimize(with_newline);
        assert!(out.ends_with('\n'));

        let without_newline = "    movq\t%rax, %rax";
        let out2 = Peephole::optimize(without_newline);
        assert!(!out2.ends_with('\n'));
    }

    #[test]
    fn cascading_removals_reach_fixpoint() {
        // Deleting the self-move exposes a store/reload pair that should
        // also collapse in the same call.
        let asm = "    movq\t%rax, %rax\n    movq\t%r10, -8(%rbp)\n    movq\t-8(%rbp), %r10\n";
        let out = Peephole::optimize(asm);
        let remaining: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(remaining, vec!["    movq\t%r10, -8(%rbp)"]);
    }
}
