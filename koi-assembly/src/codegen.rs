use crate::abi::AMD64ABI;
use crate::ir_parser::{BasicBlock, IRFunction, IRProgram, Instruction};
use crate::register_allocator::{FunctionLayout, LinearScanAllocator, ValueLocation};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone)]
struct EdgeMove {
    src: String,
    dst: String,
}

pub struct X86Generator {
    allocator: LinearScanAllocator,
    output: String,
    string_literals: BTreeMap<String, String>,
    next_string_id: usize,
    user_functions: HashSet<String>,
}

impl X86Generator {
    pub fn new() -> Self {
        X86Generator {
            allocator: LinearScanAllocator::new(),
            output: String::new(),
            string_literals: BTreeMap::new(),
            next_string_id: 0,
            user_functions: HashSet::new(),
        }
    }

    pub fn generate(&mut self, program: &IRProgram) -> Result<String, String> {
        self.user_functions = program
            .functions
            .iter()
            .map(|function| function.name.clone())
            .collect();
        self.collect_string_literals(program)?;
        self.emit_preamble();

        let layouts = Layouts::from_program(program);
        for function in &program.functions {
            let layout = self.allocator.allocate(function);
            self.generate_function(function, &layout, &layouts)?;
        }

        self.emit_line(".section .note.GNU-stack,\"\",@progbits");
        Ok(std::mem::take(&mut self.output))
    }

    fn collect_string_literals(&mut self, program: &IRProgram) -> Result<(), String> {
        for function in &program.functions {
            for block in &function.blocks {
                for instruction in &block.instructions {
                    if let Instruction::Const { value, ty, .. } = instruction
                        && ty == "string"
                    {
                        let Some(text) = value.as_str() else {
                            return Err("string constant is not a JSON string".to_string());
                        };
                        self.intern_string(text);
                    }
                }
            }
        }
        Ok(())
    }

    fn generate_function(
        &mut self,
        function: &IRFunction,
        layout: &FunctionLayout,
        layouts: &Layouts,
    ) -> Result<(), String> {
        let phi_moves = build_phi_moves(function);
        let blocks_by_label: HashMap<&str, &BasicBlock> = function
            .blocks
            .iter()
            .map(|block| (block.label.as_str(), block))
            .collect();

        self.emit_line("");
        if function.name == "main" {
            self.emit_line(".globl main");
        }
        self.emit_line(&format!("{}:", self.function_symbol(&function.name)));
        self.emit_instr("pushq", &["%rbp"]);
        self.emit_instr("movq", &["%rsp", "%rbp"]);
        if layout.stack_size > 0 {
            self.emit_instr("subq", &[&format!("${}", layout.stack_size), "%rsp"]);
        }

        self.move_parameters_to_homes(function, layout)?;
        self.emit_instr("jmp", &[&self.block_label(function, "entry")]);

        for block in &function.blocks {
            self.emit_line(&format!("{}:", self.block_label(function, &block.label)));
            for instruction in &block.instructions {
                if matches!(instruction, Instruction::Phi { .. }) {
                    continue;
                }
                match instruction {
                    Instruction::Jump { label } => {
                        self.emit_edge_moves(function, block, label, &phi_moves, layout)?;
                        self.emit_instr("jmp", &[&self.block_label(function, label)]);
                    }
                    Instruction::Branch {
                        cond,
                        true_label,
                        false_label,
                    } => {
                        self.emit_branch(function, block, cond, true_label, false_label, &phi_moves, layout)?;
                    }
                    Instruction::Return { value } => {
                        if let Some(value) = value {
                            self.load_named_value(layout, value, AMD64ABI::RETURN_REGISTER)?;
                        } else {
                            self.emit_instr("xorq", &[AMD64ABI::RETURN_REGISTER, AMD64ABI::RETURN_REGISTER]);
                        }
                        self.emit_instr("jmp", &[&self.function_end_label(function)]);
                    }
                    _ => self.generate_instruction(function, instruction, layout, layouts)?,
                }
            }
            if let Some(last) = block.instructions.last()
                && !matches!(last, Instruction::Jump { .. } | Instruction::Branch { .. } | Instruction::Return { .. })
            {
                if let Some(next) = next_block(function, &blocks_by_label, block) {
                    self.emit_edge_moves(function, block, &next.label, &phi_moves, layout)?;
                }
            }
        }

        self.emit_line(&format!("{}:", self.function_end_label(function)));
        self.emit_instr("leave", &[]);
        self.emit_instr("ret", &[]);
        Ok(())
    }

    fn move_parameters_to_homes(
        &mut self,
        function: &IRFunction,
        layout: &FunctionLayout,
    ) -> Result<(), String> {
        for (index, (name, _)) in function.parameters.iter().enumerate() {
            let dst = layout
                .locations
                .get(name)
                .ok_or_else(|| format!("no home assigned to parameter '{name}'"))?;
            if let Some(arg_reg) = AMD64ABI::arg_register(index) {
                self.store_register_to_location(arg_reg, dst);
            } else if let Some(stack_offset) = AMD64ABI::stack_arg_offset(index) {
                let src = format!("{stack_offset}(%rbp)");
                self.move_operand_to_location(&src, dst);
            }
        }
        Ok(())
    }

    fn emit_branch(
        &mut self,
        function: &IRFunction,
        block: &BasicBlock,
        cond: &str,
        true_label: &str,
        false_label: &str,
        phi_moves: &HashMap<(String, String), Vec<EdgeMove>>,
        layout: &FunctionLayout,
    ) -> Result<(), String> {
        self.load_named_value(layout, cond, AMD64ABI::SCRATCH2)?;
        self.emit_instr("cmpq", &["$0", AMD64ABI::SCRATCH2]);

        let true_moves = phi_moves.get(&(block.label.clone(), true_label.to_string()));
        let false_moves = phi_moves.get(&(block.label.clone(), false_label.to_string()));

        if true_moves.is_none() && false_moves.is_none() {
            self.emit_instr("jne", &[&self.block_label(function, true_label)]);
            self.emit_instr("jmp", &[&self.block_label(function, false_label)]);
            return Ok(());
        }

        let true_edge_label = format!("{}.branch_true", self.block_label(function, &block.label));
        self.emit_instr("jne", &[&true_edge_label]);

        self.emit_edge_moves(function, block, false_label, phi_moves, layout)?;
        self.emit_instr("jmp", &[&self.block_label(function, false_label)]);

        self.emit_line(&format!("{true_edge_label}:"));
        self.emit_edge_moves(function, block, true_label, phi_moves, layout)?;
        self.emit_instr("jmp", &[&self.block_label(function, true_label)]);
        Ok(())
    }

    fn emit_edge_moves(
        &mut self,
        _function: &IRFunction,
        block: &BasicBlock,
        successor: &str,
        phi_moves: &HashMap<(String, String), Vec<EdgeMove>>,
        layout: &FunctionLayout,
    ) -> Result<(), String> {
        let Some(moves) = phi_moves.get(&(block.label.clone(), successor.to_string())) else {
            return Ok(());
        };
        self.emit_parallel_moves(layout, moves)
    }

    fn emit_parallel_moves(
        &mut self,
        layout: &FunctionLayout,
        moves: &[EdgeMove],
    ) -> Result<(), String> {
        let mut pending: Vec<EdgeMove> = moves
            .iter()
            .filter(|edge_move| edge_move.src != edge_move.dst)
            .cloned()
            .collect();

        while !pending.is_empty() {
            let source_names: HashSet<String> = pending.iter().map(|edge_move| edge_move.src.clone()).collect();
            if let Some(index) = pending.iter().position(|edge_move| !source_names.contains(&edge_move.dst)) {
                let edge_move = pending.remove(index);
                self.move_named_value(layout, &edge_move.src, &edge_move.dst)?;
                continue;
            }

            let cycle = pending.remove(0);
            let dst = layout_home(layout, &cycle.dst)?;
            self.load_named_value(layout, &cycle.dst, AMD64ABI::SCRATCH1)?;
            self.move_named_value(layout, &cycle.src, &cycle.dst)?;
            for edge_move in &mut pending {
                if edge_move.src == cycle.dst {
                    edge_move.src = "$phi_temp".to_string();
                }
            }
            self.store_register_to_location(AMD64ABI::SCRATCH1, dst);
            while let Some(index) = pending.iter().position(|edge_move| edge_move.src == "$phi_temp") {
                let edge_move = pending.remove(index);
                let dst = layout_home(layout, &edge_move.dst)?;
                self.store_register_to_location(AMD64ABI::SCRATCH1, dst);
            }
        }

        Ok(())
    }

    fn generate_instruction(
        &mut self,
        _function: &IRFunction,
        instruction: &Instruction,
        layout: &FunctionLayout,
        layouts: &Layouts,
    ) -> Result<(), String> {
        match instruction {
            Instruction::Const { result, value, ty } => self.emit_const(layout, result, value, ty),
            Instruction::BinOp {
                result,
                lhs,
                rhs,
                op_type,
                ty,
            } => self.emit_binop(layout, result, lhs, rhs, op_type, ty),
            Instruction::Call {
                result,
                function,
                arguments,
                ..
            } => self.emit_call(layout, function, arguments, result.as_deref()),
            Instruction::CallIndirect {
                result,
                function_value,
                arguments,
                ..
            } => self.emit_call_indirect(layout, function_value, arguments, result.as_deref()),
            Instruction::Alloc { result, ty, size } => self.emit_alloc(layout, layouts, result, ty, size.as_deref()),
            Instruction::GetField {
                result,
                object,
                field,
                ty,
            } => self.emit_get_field(layout, layouts, result, object, field, ty),
            Instruction::GetIndex {
                result,
                array,
                index,
                ty,
            } => self.emit_get_index(layout, result, array, index, ty),
            Instruction::AddrOf { result, operand, .. } => self.emit_addr_of(layout, result, operand),
            Instruction::Deref { result, operand, .. } => self.emit_deref(layout, result, operand),
            Instruction::Phi { .. }
            | Instruction::Jump { .. }
            | Instruction::Branch { .. }
            | Instruction::Return { .. } => Ok(()),
        }
    }

    fn emit_const(
        &mut self,
        layout: &FunctionLayout,
        result: &str,
        value: &Value,
        ty: &str,
    ) -> Result<(), String> {
        let dst = layout_home(layout, result)?;
        match ty {
            "i64" => {
                let number = value.as_i64().ok_or_else(|| format!("const '{result}' is not an i64"))?;
                self.move_operand_to_location(&format!("${number}"), dst);
            }
            "bool" => {
                let bit = if value.as_bool().unwrap_or(false) { 1 } else { 0 };
                self.move_operand_to_location(&format!("${bit}"), dst);
            }
            "string" => {
                let text = value.as_str().ok_or_else(|| format!("const '{result}' is not a string"))?;
                let label = self.intern_string(text);
                self.emit_instr("leaq", &[&format!("{label}(%rip)"), AMD64ABI::SCRATCH2]);
                self.store_register_to_location(AMD64ABI::SCRATCH2, dst);
            }
            "f64" => {
                return Err(format!("f64 backend support is not implemented for constant '{result}'"));
            }
            _ => {
                if value.is_null() {
                    self.move_operand_to_location("$0", dst);
                } else if let Some(number) = value.as_i64() {
                    self.move_operand_to_location(&format!("${number}"), dst);
                } else if let Some(boolean) = value.as_bool() {
                    self.move_operand_to_location(if boolean { "$1" } else { "$0" }, dst);
                } else {
                    return Err(format!("unsupported constant value for '{result}': {value}"));
                }
            }
        }
        Ok(())
    }

    fn emit_binop(
        &mut self,
        layout: &FunctionLayout,
        result: &str,
        lhs: &str,
        rhs: &str,
        op_type: &str,
        ty: &str,
    ) -> Result<(), String> {
        if ty == "f64" {
            return Err(format!("f64 backend support is not implemented for binop '{op_type}'"));
        }

        self.load_named_value(layout, lhs, AMD64ABI::SCRATCH2)?;
        self.load_named_value(layout, rhs, AMD64ABI::SCRATCH0)?;
        match op_type {
            "+" => self.emit_instr("addq", &[AMD64ABI::SCRATCH0, AMD64ABI::SCRATCH2]),
            "-" => self.emit_instr("subq", &[AMD64ABI::SCRATCH0, AMD64ABI::SCRATCH2]),
            "*" => self.emit_instr("imulq", &[AMD64ABI::SCRATCH0, AMD64ABI::SCRATCH2]),
            "/" => {
                self.emit_instr("cqto", &[]);
                self.emit_instr("idivq", &[AMD64ABI::SCRATCH0]);
            }
            "<" | "<=" | ">" | ">=" | "==" | "!=" => {
                self.emit_instr("cmpq", &[AMD64ABI::SCRATCH0, AMD64ABI::SCRATCH2]);
                self.emit_setcc(op_type);
            }
            "&&" => {
                self.normalize_bool_in_place(AMD64ABI::SCRATCH2);
                self.normalize_bool_in_place(AMD64ABI::SCRATCH0);
                self.emit_instr("andq", &[AMD64ABI::SCRATCH0, AMD64ABI::SCRATCH2]);
            }
            "||" => {
                self.normalize_bool_in_place(AMD64ABI::SCRATCH2);
                self.normalize_bool_in_place(AMD64ABI::SCRATCH0);
                self.emit_instr("orq", &[AMD64ABI::SCRATCH0, AMD64ABI::SCRATCH2]);
                self.normalize_bool_in_place(AMD64ABI::SCRATCH2);
            }
            other => return Err(format!("unsupported binop '{other}'")),
        }
        let dst = layout_home(layout, result)?;
        self.store_register_to_location(AMD64ABI::SCRATCH2, dst);
        Ok(())
    }

    fn emit_call(
        &mut self,
        layout: &FunctionLayout,
        function: &str,
        arguments: &[String],
        result: Option<&str>,
    ) -> Result<(), String> {
        if function == "print" {
            self.emit_print(layout, arguments, result)?;
            return Ok(());
        }

        self.prepare_call_arguments(layout, arguments)?;
        let target = self.call_target_symbol(function);
        self.emit_instr("call", &[&target]);
        self.finish_call(arguments.len());
        if let Some(result) = result {
            let dst = layout_home(layout, result)?;
            self.store_register_to_location(AMD64ABI::RETURN_REGISTER, dst);
        }
        Ok(())
    }

    fn emit_call_indirect(
        &mut self,
        layout: &FunctionLayout,
        function_value: &str,
        arguments: &[String],
        result: Option<&str>,
    ) -> Result<(), String> {
        self.prepare_call_arguments(layout, arguments)?;
        self.load_named_value(layout, function_value, AMD64ABI::SCRATCH2)?;
        self.emit_instr("call", &[&format!("*{}", AMD64ABI::SCRATCH2)]);
        self.finish_call(arguments.len());
        if let Some(result) = result {
            let dst = layout_home(layout, result)?;
            self.store_register_to_location(AMD64ABI::RETURN_REGISTER, dst);
        }
        Ok(())
    }

    fn emit_alloc(
        &mut self,
        layout: &FunctionLayout,
        layouts: &Layouts,
        result: &str,
        ty: &str,
        size: Option<&str>,
    ) -> Result<(), String> {
        if let Some(size_value) = size {
            self.load_named_value(layout, size_value, "%rdi")?;
        } else {
            let bytes = layouts.allocation_size(ty);
            self.emit_instr("movq", &[&format!("${bytes}"), "%rdi"]);
        }
        self.emit_instr("call", &["malloc"]);
        let dst = layout_home(layout, result)?;
        self.store_register_to_location(AMD64ABI::RETURN_REGISTER, dst);
        Ok(())
    }

    fn emit_get_field(
        &mut self,
        layout: &FunctionLayout,
        layouts: &Layouts,
        result: &str,
        object: &str,
        field: &str,
        ty: &str,
    ) -> Result<(), String> {
        let object_ty = layout
            .value_types
            .get(object)
            .ok_or_else(|| format!("unknown type for field-access object '{object}'"))?;
        let offset = layouts.field_offset(object_ty, field);
        self.load_named_value(layout, object, AMD64ABI::SCRATCH2)?;
        self.emit_instr("movq", &[&format!("{offset}({})", AMD64ABI::SCRATCH2), AMD64ABI::SCRATCH0]);
        let dst = layout_home(layout, result)?;
        self.store_register_to_location(AMD64ABI::SCRATCH0, dst);
        if ty == "f64" {
            return Err("f64 backend support is not implemented for field access".to_string());
        }
        Ok(())
    }

    fn emit_get_index(
        &mut self,
        layout: &FunctionLayout,
        result: &str,
        array: &str,
        index: &str,
        ty: &str,
    ) -> Result<(), String> {
        let element_size = type_size(ty);
        self.load_named_value(layout, array, AMD64ABI::SCRATCH2)?;
        self.load_named_value(layout, index, AMD64ABI::SCRATCH0)?;
        if element_size != 1 {
            self.emit_instr("imulq", &[&format!("${element_size}"), AMD64ABI::SCRATCH0]);
        }
        self.emit_instr("addq", &[AMD64ABI::SCRATCH0, AMD64ABI::SCRATCH2]);
        self.emit_instr("movq", &[&format!("0({})", AMD64ABI::SCRATCH2), AMD64ABI::SCRATCH1]);
        let dst = layout_home(layout, result)?;
        self.store_register_to_location(AMD64ABI::SCRATCH1, dst);
        Ok(())
    }

    fn emit_addr_of(
        &mut self,
        layout: &FunctionLayout,
        result: &str,
        operand: &str,
    ) -> Result<(), String> {
        let src = layout_home(layout, operand)?;
        let dst = layout_home(layout, result)?;
        let ValueLocation::Stack(offset) = src;
        self.emit_instr("leaq", &[&format!("{offset}(%rbp)"), AMD64ABI::SCRATCH2]);
        self.store_register_to_location(AMD64ABI::SCRATCH2, dst);
        Ok(())
    }

    fn emit_deref(
        &mut self,
        layout: &FunctionLayout,
        result: &str,
        operand: &str,
    ) -> Result<(), String> {
        self.load_named_value(layout, operand, AMD64ABI::SCRATCH2)?;
        self.emit_instr("movq", &[&format!("0({})", AMD64ABI::SCRATCH2), AMD64ABI::SCRATCH0]);
        let dst = layout_home(layout, result)?;
        self.store_register_to_location(AMD64ABI::SCRATCH0, dst);
        Ok(())
    }

    fn emit_print(
        &mut self,
        layout: &FunctionLayout,
        arguments: &[String],
        result: Option<&str>,
    ) -> Result<(), String> {
        if arguments.len() != 1 {
            return Err("print currently expects exactly one argument".to_string());
        }
        let argument = &arguments[0];
        let format_label = match layout.value_types.get(argument).map(String::as_str) {
            Some("string") => ".LC_print_string",
            _ => ".LC_print_i64",
        };
        self.emit_instr("leaq", &[&format!("{format_label}(%rip)"), "%rdi"]);
        self.load_named_value(layout, argument, "%rsi")?;
        self.emit_instr("xorq", &["%rax", "%rax"]);
        self.emit_instr("call", &["printf"]);
        if let Some(result) = result {
            let dst = layout_home(layout, result)?;
            self.store_register_to_location(AMD64ABI::RETURN_REGISTER, dst);
        }
        Ok(())
    }

    fn prepare_call_arguments(
        &mut self,
        layout: &FunctionLayout,
        arguments: &[String],
    ) -> Result<(), String> {
        let stack_arg_count = arguments.len().saturating_sub(AMD64ABI::ARG_REGISTERS.len());
        let stack_arg_bytes = (stack_arg_count as i64) * 8;
        let stack_space = AMD64ABI::align_to_16(stack_arg_bytes);
        if stack_space > 0 {
            self.emit_instr("subq", &[&format!("${stack_space}"), "%rsp"]);
        }

        for (index, argument) in arguments.iter().enumerate().skip(AMD64ABI::ARG_REGISTERS.len()) {
            self.load_named_value(layout, argument, AMD64ABI::SCRATCH2)?;
            let slot = ((index - AMD64ABI::ARG_REGISTERS.len()) * 8) as i64;
            self.emit_instr("movq", &[AMD64ABI::SCRATCH2, &format!("{slot}(%rsp)")]);
        }

        for (index, argument) in arguments.iter().take(AMD64ABI::ARG_REGISTERS.len()).enumerate() {
            let arg_reg = AMD64ABI::arg_register(index).expect("bounded by take");
            self.load_named_value(layout, argument, arg_reg)?;
        }

        Ok(())
    }

    fn finish_call(&mut self, arg_count: usize) {
        let stack_arg_count = arg_count.saturating_sub(AMD64ABI::ARG_REGISTERS.len());
        let stack_space = AMD64ABI::align_to_16((stack_arg_count as i64) * 8);
        if stack_space > 0 {
            self.emit_instr("addq", &[&format!("${stack_space}"), "%rsp"]);
        }
    }

    fn move_named_value(
        &mut self,
        layout: &FunctionLayout,
        src_name: &str,
        dst_name: &str,
    ) -> Result<(), String> {
        let dst = layout_home(layout, dst_name)?;
        if let Some(src) = layout.locations.get(src_name) {
            self.move_location_to_location(src, dst);
        } else if self.user_functions.contains(src_name) {
            let symbol = self.function_symbol(src_name);
            self.emit_instr("leaq", &[&format!("{symbol}(%rip)"), AMD64ABI::SCRATCH2]);
            self.store_register_to_location(AMD64ABI::SCRATCH2, dst);
        } else {
            return Err(format!("no home allocated for value '{src_name}'"));
        }
        Ok(())
    }

    fn load_named_value(
        &mut self,
        layout: &FunctionLayout,
        name: &str,
        register: &str,
    ) -> Result<(), String> {
        if let Some(src) = layout.locations.get(name) {
            self.emit_instr("movq", &[&src.as_operand(), register]);
        } else if self.user_functions.contains(name) {
            let symbol = self.function_symbol(name);
            self.emit_instr("leaq", &[&format!("{symbol}(%rip)"), register]);
        } else {
            return Err(format!("no home allocated for value '{name}'"));
        }
        Ok(())
    }

    fn move_location_to_location(&mut self, src: &ValueLocation, dst: &ValueLocation) {
        self.emit_instr("movq", &[&src.as_operand(), AMD64ABI::SCRATCH2]);
        self.store_register_to_location(AMD64ABI::SCRATCH2, dst);
    }

    fn move_operand_to_location(&mut self, operand: &str, dst: &ValueLocation) {
        self.emit_instr("movq", &[operand, AMD64ABI::SCRATCH2]);
        self.store_register_to_location(AMD64ABI::SCRATCH2, dst);
    }

    fn store_register_to_location(&mut self, register: &str, dst: &ValueLocation) {
        self.emit_instr("movq", &[register, &dst.as_operand()]);
    }

    fn normalize_bool_in_place(&mut self, register: &str) {
        self.emit_instr("cmpq", &["$0", register]);
        self.emit_instr("setne", &["%al"]);
        self.emit_instr("movzbq", &["%al", register]);
    }

    fn emit_setcc(&mut self, op_type: &str) {
        let mnemonic = match op_type {
            "<" => "setl",
            "<=" => "setle",
            ">" => "setg",
            ">=" => "setge",
            "==" => "sete",
            "!=" => "setne",
            _ => unreachable!("validated by caller"),
        };
        self.emit_instr(mnemonic, &["%al"]);
        self.emit_instr("movzbq", &["%al", AMD64ABI::SCRATCH2]);
    }

    fn intern_string(&mut self, text: &str) -> String {
        if let Some(existing) = self.string_literals.get(text) {
            return existing.clone();
        }
        let label = format!(".LC_str_{}", self.next_string_id);
        self.next_string_id += 1;
        self.string_literals.insert(text.to_string(), label.clone());
        label
    }

    fn emit_preamble(&mut self) {
        self.emit_line(".section .rodata");
        self.emit_line(".LC_print_i64:");
        self.emit_line("    .string \"%ld\\n\"");
        self.emit_line(".LC_print_string:");
        self.emit_line("    .string \"%s\\n\"");
        for (text, label) in self.string_literals.clone() {
            self.emit_line(&format!("{label}:"));
            self.emit_line(&format!("    .string {:?}", text));
        }
        self.emit_line(".text");
    }

    fn emit_instr(&mut self, op: &str, args: &[&str]) {
        if args.is_empty() {
            self.output.push_str(&format!("    {op}\n"));
        } else {
            self.output
                .push_str(&format!("    {op}\t{}\n", args.join(", ")));
        }
    }

    fn emit_line(&mut self, line: &str) {
        self.output.push_str(line);
        self.output.push('\n');
    }

    fn block_label(&self, function: &IRFunction, block: &str) -> String {
        format!(
            ".L{}_{}",
            sanitize_symbol(&function.name),
            sanitize_symbol(block)
        )
    }

    fn function_end_label(&self, function: &IRFunction) -> String {
        format!(".L{}_end", sanitize_symbol(&function.name))
    }

    fn function_symbol(&self, function_name: &str) -> String {
        sanitize_symbol(function_name)
    }

    fn call_target_symbol(&self, function_name: &str) -> String {
        if self.user_functions.contains(function_name) {
            self.function_symbol(function_name)
        } else {
            function_name.to_string()
        }
    }
}

fn layout_home<'a>(layout: &'a FunctionLayout, name: &str) -> Result<&'a ValueLocation, String> {
    layout
        .locations
        .get(name)
        .ok_or_else(|| format!("no home allocated for value '{name}'"))
}

fn next_block<'a>(
    function: &'a IRFunction,
    _blocks_by_label: &HashMap<&'a str, &'a BasicBlock>,
    current: &'a BasicBlock,
) -> Option<&'a BasicBlock> {
    let mut iter = function.blocks.iter();
    while let Some(block) = iter.next() {
        if block.label == current.label {
            return iter.next();
        }
    }
    None
}

fn build_phi_moves(function: &IRFunction) -> HashMap<(String, String), Vec<EdgeMove>> {
    let mut moves = HashMap::<(String, String), Vec<EdgeMove>>::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let Instruction::Phi { result, incoming, .. } = instruction {
                for (pred, value) in incoming {
                    moves
                        .entry((pred.clone(), block.label.clone()))
                        .or_default()
                        .push(EdgeMove {
                            src: value.clone(),
                            dst: result.clone(),
                        });
                }
            }
        }
    }
    moves
}

#[derive(Debug, Clone, Default)]
struct Layouts {
    field_offsets: HashMap<(String, String), i64>,
    struct_sizes: HashMap<String, i64>,
}

impl Layouts {
    fn from_program(program: &IRProgram) -> Self {
        let mut layouts = Layouts::default();

        for function in &program.functions {
            let value_types = collect_value_types(function);
            for block in &function.blocks {
                for instruction in &block.instructions {
                    if let Instruction::GetField { object, field, .. } = instruction
                        && let Some(object_ty) = value_types.get(object)
                        && !is_pointer_like_scalar(object_ty)
                    {
                        let key = (object_ty.clone(), field.clone());
                        if !layouts.field_offsets.contains_key(&key) {
                            let next_index = layouts
                                .field_offsets
                                .keys()
                                .filter(|(struct_name, _)| struct_name == object_ty)
                                .count() as i64;
                            layouts.field_offsets.insert(key, next_index * 8);
                            layouts
                                .struct_sizes
                                .insert(object_ty.clone(), (next_index + 1) * 8);
                        }
                    }
                }
            }
        }

        layouts
    }

    fn field_offset(&self, struct_name: &str, field: &str) -> i64 {
        self.field_offsets
            .get(&(struct_name.to_string(), field.to_string()))
            .copied()
            .unwrap_or(0)
    }

    fn allocation_size(&self, ty: &str) -> i64 {
        if let Some(size) = self.struct_sizes.get(ty) {
            return (*size).max(8);
        }
        if ty.starts_with("arr_") {
            return 64;
        }
        type_size(ty).max(8)
    }
}

fn collect_value_types(function: &IRFunction) -> HashMap<String, String> {
    let mut value_types = HashMap::new();
    for (name, ty) in &function.parameters {
        value_types.insert(name.clone(), ty.clone());
    }
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let (Some(result), Some(ty)) = (instruction.result_name(), instruction.result_type()) {
                value_types.insert(result.to_string(), ty.to_string());
            }
        }
    }
    value_types
}

fn is_pointer_like_scalar(ty: &str) -> bool {
    matches!(ty, "i64" | "f64" | "bool" | "string") || ty.starts_with("ptr_")
}

fn type_size(ty: &str) -> i64 {
    match ty {
        "f64" => 8,
        "i64" | "bool" | "string" => 8,
        _ if ty.starts_with("ptr_") => 8,
        _ if ty.starts_with("arr_") => 8,
        _ if ty.starts_with("fn_") => 8,
        _ => 8,
    }
}

fn sanitize_symbol(name: &str) -> String {
    let mut symbol = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            symbol.push(ch);
        } else {
            symbol.push('_');
        }
    }
    if symbol.is_empty() {
        "_".to_string()
    } else if symbol.as_bytes()[0].is_ascii_digit() {
        format!("_{symbol}")
    } else {
        symbol
    }
}
