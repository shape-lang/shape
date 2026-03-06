//! VM Debugger for Shape Virtual Machine
//!
//! Provides comprehensive debugging and tracing capabilities including:
//! - Instruction-level tracing
//! - Breakpoint support
//! - Stack inspection
//! - Call stack visualization
//! - Step-by-step execution
//! - Variable name resolution

use std::io::{self, Write};

use super::{
    bytecode::{BytecodeProgram, Instruction, Operand},
    executor::DebugVMState,
};
/// Debugger commands
#[derive(Debug, Clone)]
pub enum DebugCommand {
    /// Continue execution
    Continue,
    /// Step to next instruction
    Step,
    /// Step into function calls
    StepInto,
    /// Step over function calls
    StepOver,
    /// Step out of current function
    StepOut,
    /// Print current stack
    Stack,
    /// Print local variables
    Locals,
    /// Print module binding variables
    ModuleBindings,
    /// Print call stack
    CallStack,
    /// Set breakpoint at instruction
    Breakpoint(usize),
    /// Remove breakpoint
    ClearBreakpoint(usize),
    /// List all breakpoints
    ListBreakpoints,
    /// Print current instruction
    CurrentInstruction,
    /// Print next N instructions
    Disassemble(usize),
    /// Print variable value
    Print(String),
    /// Show help
    Help,
    /// Quit debugger
    Quit,
}

/// Debugger state
#[derive(Debug)]
pub struct DebuggerState {
    /// Breakpoints (instruction indices)
    breakpoints: Vec<usize>,
    /// Whether to trace all instructions
    trace_mode: bool,
    /// Step mode (step into, over, out)
    step_mode: StepMode,
    /// Call depth when stepping out
    step_out_depth: usize,
    /// Whether debugger is active
    active: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum StepMode {
    None,
    Into,
    Over,
    Out,
}

impl Default for DebuggerState {
    fn default() -> Self {
        Self {
            breakpoints: Vec::new(),
            trace_mode: false,
            step_mode: StepMode::None,
            step_out_depth: 0,
            active: false,
        }
    }
}

/// VM Debugger
pub struct VMDebugger {
    state: DebuggerState,
}

impl VMDebugger {
    /// Create a new debugger
    pub fn new() -> Self {
        Self {
            state: DebuggerState::default(),
        }
    }

    /// Start debugging session
    pub fn start(&mut self) {
        self.state.active = true;
        println!("🐛 Shape VM Debugger started");
        println!("Type 'help' for available commands");
    }

    /// Check if debugger should break at current instruction
    pub fn should_break(&mut self, vm_state: &DebugVMState, ip: usize) -> bool {
        if !self.state.active {
            return false;
        }

        // Check breakpoints
        if self.state.breakpoints.contains(&ip) {
            println!("🔴 Breakpoint hit at instruction {}", ip);
            return true;
        }

        // Check step mode
        match self.state.step_mode {
            StepMode::Into => {
                self.state.step_mode = StepMode::None;
                return true;
            }
            StepMode::Over => {
                // Step over means break at next instruction at same call depth
                if vm_state.call_stack_depth <= self.state.step_out_depth {
                    self.state.step_mode = StepMode::None;
                    return true;
                }
            }
            StepMode::Out => {
                // Step out means break when call depth decreases
                if vm_state.call_stack_depth < self.state.step_out_depth {
                    self.state.step_mode = StepMode::None;
                    return true;
                }
            }
            StepMode::None => {}
        }

        false
    }

    /// Handle debug break
    pub fn debug_break(&mut self, vm_state: &DebugVMState, program: &BytecodeProgram) {
        println!("\n📍 Debug break at instruction {}", vm_state.ip);
        self.print_current_state(vm_state, program);

        loop {
            print!("(shape-debug) ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            if io::stdin().read_line(&mut input).is_err() {
                break;
            }

            let command = self.parse_command(input.trim());
            if self.execute_command(command, vm_state, program) {
                break; // Continue execution
            }
        }
    }

    /// Enable/disable trace mode
    pub fn set_trace_mode(&mut self, enabled: bool) {
        self.state.trace_mode = enabled;
        if enabled {
            println!("📊 Instruction tracing enabled");
        } else {
            println!("📊 Instruction tracing disabled");
        }
    }

    /// Trace instruction execution
    pub fn trace_instruction(
        &self,
        vm_state: &DebugVMState,
        program: &BytecodeProgram,
        instruction: &Instruction,
    ) {
        if !self.state.trace_mode {
            return;
        }

        let ip = vm_state.ip;
        let line_info = self.get_line_info(program, ip);

        print!("[{}] {:04X}: ", line_info, ip);
        self.print_instruction(instruction, program);
        println!();
    }

    /// Parse debugger command
    fn parse_command(&self, input: &str) -> DebugCommand {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() {
            return DebugCommand::Help;
        }

        match parts[0] {
            "c" | "continue" => DebugCommand::Continue,
            "s" | "step" => DebugCommand::Step,
            "si" | "stepi" | "into" => DebugCommand::StepInto,
            "so" | "over" => DebugCommand::StepOver,
            "out" => DebugCommand::StepOut,
            "stack" => DebugCommand::Stack,
            "locals" => DebugCommand::Locals,
            "module_bindings" | "bindings" => DebugCommand::ModuleBindings,
            "callstack" | "bt" => DebugCommand::CallStack,
            "b" | "break" => {
                if parts.len() > 1 {
                    if let Ok(addr) = parts[1].parse::<usize>() {
                        DebugCommand::Breakpoint(addr)
                    } else {
                        DebugCommand::Help
                    }
                } else {
                    DebugCommand::ListBreakpoints
                }
            }
            "clear" => {
                if parts.len() > 1 {
                    if let Ok(addr) = parts[1].parse::<usize>() {
                        DebugCommand::ClearBreakpoint(addr)
                    } else {
                        DebugCommand::Help
                    }
                } else {
                    DebugCommand::Help
                }
            }
            "list" => DebugCommand::ListBreakpoints,
            "inst" | "instruction" => DebugCommand::CurrentInstruction,
            "dis" | "disasm" => {
                let count = if parts.len() > 1 {
                    parts[1].parse().unwrap_or(10)
                } else {
                    10
                };
                DebugCommand::Disassemble(count)
            }
            "p" | "print" => {
                if parts.len() > 1 {
                    DebugCommand::Print(parts[1..].join(" "))
                } else {
                    DebugCommand::Help
                }
            }
            "h" | "help" => DebugCommand::Help,
            "q" | "quit" => DebugCommand::Quit,
            _ => DebugCommand::Help,
        }
    }

    /// Execute debugger command
    fn execute_command(
        &mut self,
        command: DebugCommand,
        vm_state: &DebugVMState,
        program: &BytecodeProgram,
    ) -> bool {
        match command {
            DebugCommand::Continue => {
                self.state.step_mode = StepMode::None;
                return true;
            }
            DebugCommand::Step | DebugCommand::StepInto => {
                self.state.step_mode = StepMode::Into;
                return true;
            }
            DebugCommand::StepOver => {
                self.state.step_mode = StepMode::Over;
                self.state.step_out_depth = vm_state.call_stack_depth;
                return true;
            }
            DebugCommand::StepOut => {
                self.state.step_mode = StepMode::Out;
                self.state.step_out_depth = vm_state.call_stack_depth;
                return true;
            }
            DebugCommand::Stack => self.print_stack_placeholder(),
            DebugCommand::Locals => self.print_locals_placeholder(),
            DebugCommand::ModuleBindings => self.print_module_bindings_placeholder(),
            DebugCommand::CallStack => self.print_call_stack(vm_state),
            DebugCommand::Breakpoint(addr) => {
                if !self.state.breakpoints.contains(&addr) {
                    self.state.breakpoints.push(addr);
                    println!("🔴 Breakpoint set at instruction {}", addr);
                } else {
                    println!("Breakpoint already exists at instruction {}", addr);
                }
            }
            DebugCommand::ClearBreakpoint(addr) => {
                if let Some(pos) = self.state.breakpoints.iter().position(|&x| x == addr) {
                    self.state.breakpoints.remove(pos);
                    println!("🟢 Breakpoint removed from instruction {}", addr);
                } else {
                    println!("No breakpoint at instruction {}", addr);
                }
            }
            DebugCommand::ListBreakpoints => self.list_breakpoints(),
            DebugCommand::CurrentInstruction => self.print_current_instruction(vm_state, program),
            DebugCommand::Disassemble(count) => self.disassemble(vm_state, program, count),
            DebugCommand::Print(var) => self.print_variable_placeholder(&var),
            DebugCommand::Help => self.print_help(),
            DebugCommand::Quit => {
                self.state.active = false;
                println!("👋 Debugger stopped");
                return true;
            }
        }
        false
    }

    /// Print current VM state
    fn print_current_state(&self, vm_state: &DebugVMState, program: &BytecodeProgram) {
        let ip = vm_state.ip;
        let line_info = self.get_line_info(program, ip);

        println!("📍 Position: {} (instruction {})", line_info, ip);

        if let Some(instruction) = program.instructions.get(ip) {
            print!("📜 Current: ");
            self.print_instruction(instruction, program);
            println!();
        }

        // Show call depth
        println!("📞 Call depth: {}", vm_state.call_stack_depth);
    }

    /// Print stack contents (placeholder - requires full VM access)
    fn print_stack_placeholder(&self) {
        println!("📚 Stack contents:");
        println!(
            "  (Stack inspection requires full VM access - not available in current debug mode)"
        );
    }

    /// Print local variables (placeholder - requires full VM access)
    fn print_locals_placeholder(&self) {
        println!("🏠 Local variables:");
        println!(
            "  (Variable inspection requires full VM access - not available in current debug mode)"
        );
    }

    /// Print module binding variables (placeholder - requires full VM access)
    fn print_module_bindings_placeholder(&self) {
        println!("🌍 Module binding variables:");
        println!(
            "  (Variable inspection requires full VM access - not available in current debug mode)"
        );
    }

    /// Print call stack
    fn print_call_stack(&self, vm_state: &DebugVMState) {
        println!("📞 Call stack:");
        println!("  Current depth: {}", vm_state.call_stack_depth);
        println!(
            "  (Detailed call stack requires full VM access - not available in current debug mode)"
        );
    }

    /// List all breakpoints
    fn list_breakpoints(&self) {
        println!("🔴 Breakpoints:");
        if self.state.breakpoints.is_empty() {
            println!("  (none)");
        } else {
            for &bp in &self.state.breakpoints {
                println!("  {}", bp);
            }
        }
    }

    /// Print current instruction
    fn print_current_instruction(&self, vm_state: &DebugVMState, program: &BytecodeProgram) {
        let ip = vm_state.ip;
        if let Some(instruction) = program.instructions.get(ip) {
            let line_info = self.get_line_info(program, ip);
            print!("📜 {} [{:04X}]: ", line_info, ip);
            self.print_instruction(instruction, program);
            println!();
        } else {
            println!("No instruction at current position");
        }
    }

    /// Disassemble instructions
    fn disassemble(&self, vm_state: &DebugVMState, program: &BytecodeProgram, count: usize) {
        let start_ip = vm_state.ip;
        println!("📜 Disassembly from instruction {}:", start_ip);

        for i in 0..count {
            let ip = start_ip + i;
            if let Some(instruction) = program.instructions.get(ip) {
                let line_info = self.get_line_info(program, ip);
                let marker = if ip == start_ip { ">" } else { " " };
                print!("{} {} [{:04X}]: ", marker, line_info, ip);
                self.print_instruction(instruction, program);
                println!();
            } else {
                break;
            }
        }
    }

    /// Print variable value (placeholder - requires full VM access)
    fn print_variable_placeholder(&self, var_name: &str) {
        println!(
            "❌ Variable inspection for '{}' requires full VM access - not available in current debug mode",
            var_name
        );
    }

    /// Print help information
    fn print_help(&self) {
        println!("🐛 Shape VM Debugger Commands:");
        println!("  c, continue       - Continue execution");
        println!("  s, step           - Step to next instruction");
        println!("  si, into, stepi   - Step into function calls");
        println!("  so, over          - Step over function calls");
        println!("  out               - Step out of current function");
        println!("  stack             - Print stack contents");
        println!("  locals            - Print local variables");
        println!("  module_bindings           - Print module binding variables");
        println!("  callstack, bt     - Print call stack");
        println!("  b, break <addr>   - Set breakpoint at instruction");
        println!("  clear <addr>      - Remove breakpoint");
        println!("  list              - List all breakpoints");
        println!("  inst, instruction - Print current instruction");
        println!("  dis, disasm [N]   - Disassemble N instructions (default 10)");
        println!("  p, print <var>    - Print variable value");
        println!("  h, help           - Show this help");
        println!("  q, quit           - Stop debugger and continue");
    }

    // Helper methods

    fn print_instruction(&self, instruction: &Instruction, program: &BytecodeProgram) {
        print!("{:?}", instruction.opcode);

        if let Some(ref operand) = instruction.operand {
            match operand {
                Operand::Const(idx) => {
                    if let Some(constant) = program.constants.get(*idx as usize) {
                        print!(" {:?}", constant);
                    } else {
                        print!(" const[{}]", idx);
                    }
                }
                Operand::Local(idx) => {
                    let name = self.get_variable_name(program, *idx, false);
                    print!(" {}", name);
                }
                Operand::ModuleBinding(idx) => {
                    let name = self.get_variable_name(program, *idx, true);
                    print!(" {}", name);
                }
                Operand::Function(idx) => {
                    if let Some(func) = program.functions.get(idx.index()) {
                        print!(" {}()", func.name);
                    } else {
                        print!(" func[{}]", idx);
                    }
                }
                Operand::Property(idx) => {
                    if let Some(prop) = program.strings.get(*idx as usize) {
                        print!(" .{}", prop);
                    } else {
                        print!(" prop[{}]", idx);
                    }
                }
                _ => print!(" {:?}", operand),
            }
        }
    }

    fn get_line_info(&self, program: &BytecodeProgram, ip: usize) -> String {
        let debug_info = &program.debug_info;
        // Find the file and line number for this instruction
        if let Some((file_id, line_num)) = debug_info.get_location_for_instruction(ip) {
            let file_name = debug_info
                .source_map
                .get_file(file_id)
                .unwrap_or("<unknown>");
            if file_name.is_empty() || file_name == "<main>" {
                format!("line {}", line_num)
            } else {
                format!("{}:{}", file_name, line_num)
            }
        } else {
            "unknown".to_string()
        }
    }

    fn get_variable_name(&self, program: &BytecodeProgram, index: u16, is_global: bool) -> String {
        let debug_info = &program.debug_info;
        for (var_index, name) in &debug_info.variable_names {
            if *var_index == index {
                return name.clone();
            }
        }

        if is_global {
            format!("module_binding[{}]", index)
        } else {
            format!("local[{}]", index)
        }
    }
}

impl Default for VMDebugger {
    fn default() -> Self {
        Self::new()
    }
}
