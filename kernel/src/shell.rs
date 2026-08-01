use syn_shell::{
    Error,
    editor::{EditorAction, Key, LineEditor},
    interpreter::{CommandExecutor, ExecutionToken, Interpreter, InterpreterEvent},
    parser::{CommandCall, CommandRegistry, RouteId},
    render::{OutputFormat, render},
};
use synos_boot_protocol::{BootInfo, BootMethod, MemoryKind};
use synos_power::AcpiPlatform;
use synos_status::Status;
use synos_system_model::command::{CommandSpec, OutputText, OutputValue, StructuredOutput};
use crate::monitor::{MonitorState, MonitorView, MAX_LOCKS};
use crate::scheduler::Scheduler;
use crate::dlm::{DistributedLockManager, NodeFenceTable, DEFAULT_LOCK_CAPACITY, DEFAULT_NODE_FENCE_CAPACITY};

const HELP_ROUTE: u16 = 1;
const SHOW_SYSTEM_ROUTE: u16 = 2;
const REBOOT_ROUTE: u16 = 3;
const SHUTDOWN_ROUTE: u16 = 4;
const MONITOR_ROUTE: u16 = 5;
const SHOW_PROCESSES_ROUTE: u16 = 6;
const TOP_CPU_ROUTE: u16 = 7;
const SHOW_MEMORY_ROUTE: u16 = 8;
const SHOW_DSM_ROUTE: u16 = 9;
const COMMAND_CAPACITY: usize = 10;
const HISTORY_CAPACITY: usize = 8;

pub fn run(
    boot_info: &'static BootInfo,
    scheduler: &'static Scheduler,
    dlm: &'static DistributedLockManager<DEFAULT_LOCK_CAPACITY>,
    node_fences: &'static NodeFenceTable<DEFAULT_NODE_FENCE_CAPACITY>,
    scheduler_clock: u64,
    acpi: Option<AcpiPlatform>,
) -> ! {
    let mut registry = CommandRegistry::<COMMAND_CAPACITY>::new();
    register(&mut registry, "HELP", HELP_ROUTE);
    register(&mut registry, "SHOW-SYSTEM", SHOW_SYSTEM_ROUTE);
    register(&mut registry, "REBOOT", REBOOT_ROUTE);
    register(&mut registry, "SHUTDOWN", SHUTDOWN_ROUTE);
    register(&mut registry, "MONITOR", MONITOR_ROUTE);
    register(&mut registry, "SHOW-PROCESSES", SHOW_PROCESSES_ROUTE);
    register(&mut registry, "TOP-CPU", TOP_CPU_ROUTE);
    register(&mut registry, "SHOW-MEMORY", SHOW_MEMORY_ROUTE);
    register(&mut registry, "SHOW-DSM", SHOW_DSM_ROUTE);

    let mut editor = LineEditor::<HISTORY_CAPACITY>::new();
    let mut interpreter = Interpreter::new();
    let mut executor =
        KernelExecutor::new(boot_info, scheduler, dlm, node_fences, scheduler_clock, acpi.is_some());
    let mut keyboard = crate::keyboard::Keyboard::new();
    let mut usb_keyboard = crate::usb_keyboard::UsbKeyboard::new();
    let mut input = VtInput::new();
    let mut ignore_line_feed = false;

    banner();
    prompt();

    loop {
        let byte = wait_for_byte(&mut keyboard, &mut usb_keyboard, acpi.as_ref());
        if ignore_line_feed && byte == b'\n' {
            ignore_line_feed = false;
            continue;
        }
        ignore_line_feed = false;

        let key = match byte {
            b'\r' => {
                ignore_line_feed = true;
                Some(Key::Enter)
            }
            _ => input.advance(byte),
        };
        let Some(key) = key else { continue };

        match editor.handle(key) {
            Ok(EditorAction::Redraw) => redraw(&editor),
            Ok(EditorAction::Complete) => {
                if let Err(error) = complete_line(&mut editor, &registry) {
                    crate::println!();
                    crate::println!("shell completion error: {error:?}");
                    redraw(&editor)
                }
            }
            Ok(EditorAction::Submit(line)) => {
                crate::println!();
                if !line.as_str().trim().is_empty() {
                    execute_line(
                        line.as_str(),
                        &registry,
                        &mut interpreter,
                        &mut executor,
                        acpi.as_ref(),
                    )
                }
                prompt()
            }
            Ok(EditorAction::Cancel) => {
                crate::println!("\x1b[91m^C\x1b[0m");
                prompt()
            }
            Ok(EditorAction::None) => {}
            Err(error) => {
                crate::println!();
                crate::println!("\x1b[91mshell input error:\x1b[0m {error:?}");
                editor.clear();
                prompt()
            }
        }
    }
}

fn register(registry: &mut CommandRegistry<COMMAND_CAPACITY>, name: &str, route: u16) {
    let command = CommandSpec::new(name, &[]).expect("valid kernel command");
    let route = RouteId::new(route).expect("nonzero kernel route");
    registry
        .register(command, route)
        .expect("kernel command registry has capacity")
}

fn execute_line(
    line: &str,
    registry: &CommandRegistry<COMMAND_CAPACITY>,
    interpreter: &mut Interpreter,
    executor: &mut KernelExecutor,
    acpi: Option<&AcpiPlatform>,
) {
    let program = match registry.parse(line) {
        Ok(program) => program,
        Err(Error::AmbiguousCommand) => {
            crate::println!("shell error: ambiguous command");
            if let Ok(suggestions) = registry.suggestions(line) {
                for command in suggestions.commands() {
                    print_command_suggestion(command.as_str());
                }
            }
            return;
        }
        Err(error) => {
            crate::println!("shell error: {error:?}");
            return;
        }
    };
    if program.background {
        crate::println!("shell error: background jobs not ready");
        return;
    }

    let (human_system_output, human_memory_output, human_dsm_output) =
        if program.stage_count() == 1 {
            match program.stage(0).map(|stage| stage.route.raw()) {
                Some(SHOW_SYSTEM_ROUTE) => (true, false, false),
                Some(SHOW_MEMORY_ROUTE) => (false, true, false),
                Some(SHOW_DSM_ROUTE) => (false, false, true),
                _ => (false, false, false),
            }
        } else {
            (false, false, false)
        };

    if human_system_output {
        executor.print_system();
        return
    }

    match interpreter.start_program(program, executor) {
        Ok(InterpreterEvent::Started) => {}
        Ok(_) => {
            crate::println!("shell error: command did not start");
            return;
        }
        Err(error) => {
            crate::println!("shell error: {error:?}");
            return;
        }
    }

    while interpreter.is_running() {
        match interpreter.poll(executor) {
            Ok(InterpreterEvent::Pending) => crate::arch::halt(),
            Ok(InterpreterEvent::Complete(output)) => {
                if !human_memory_output && !human_dsm_output {
                    match render(&output, OutputFormat::List) {
                        Ok(text) => crate::print!("{}", text.as_str()),
                        Err(error) => crate::println!("shell output error: {error:?}"),
                    }
                }
            }
            Ok(InterpreterEvent::Failed(status)) => {
                crate::println!("command failed: status={:#x}", status.raw())
            }
            Ok(InterpreterEvent::Cancelled) => crate::println!("command cancelled"),
            Ok(InterpreterEvent::Submitted(job)) => {
                crate::println!("background job {} submitted", job.raw())
            }
            Ok(InterpreterEvent::Started) => {}
            Err(error) => {
                crate::println!("shell error: {error:?}");
                break;
            }
        }
    }

    if executor.take_reboot_requested() {
        crate::power::reboot(acpi)
    }
    if executor.take_shutdown_requested() {
        crate::power::shutdown(acpi)
    }
}

fn prompt() {
    crate::print!("\x1b[1;32mSYNOS\x1b[90m::\x1b[36mROOT\x1b[0m> ")
}

fn complete_line(
    editor: &mut LineEditor<HISTORY_CAPACITY>,
    registry: &CommandRegistry<COMMAND_CAPACITY>,
) -> Result<(), Error> {
    if let Some(command) = registry.unique_suggestion(editor.line())? {
        editor.replace_command(command.as_str())?;
        redraw(editor);
        return Ok(())
    }

    let suggestions = registry.suggestions(editor.line())?;
    if suggestions.commands().count() > 1 {
        crate::println!();
        for command in suggestions.commands() {
            print_command_suggestion(command.as_str());
        }
        redraw(editor);
    }
    Ok(())
}

fn print_command_suggestion(command: &str) {
    crate::print!("  ");
    for byte in command.bytes() {
        crate::print!("{}", if byte == b'-' { ' ' } else { byte as char });
    }
    crate::println!();
}

fn redraw<const HISTORY: usize>(editor: &LineEditor<HISTORY>) {
    crate::print!("\r\x1b[2K");
    prompt();
    crate::print!("{}", editor.line());
    let tail = editor.line().len().saturating_sub(editor.cursor());
    if tail != 0 {
        crate::print!("\x1b[{tail}D")
    }
}

fn banner() {
    crate::console::clear();
    crate::println!("\x1b[1;36m{}", r"   _____             ____  _____");
    crate::println!("\x1b[1;96m{}", r"  / ___/__  ______  / __ \/ ___/");
    crate::println!("\x1b[1;94m{}", r"  \__ \/ / / / __ \/ / / /\__ \ ");
    crate::println!("\x1b[1;34m{}", r" ___/ / /_/ / / / / /_/ /___/ /");
    crate::println!("\x1b[1;35m{}", r"/____/\__, /_/ /_/\____//____/");
    crate::println!("\x1b[1;95m{}", r"     /____/                     ");
    crate::println!();
    crate::println!("\x1b[90m  SYNCHRONOUS NETWORK OPERATING SYSTEM // VT100 ONLINE\x1b[0m");
    crate::println!("\x1b[34m  --------------------------------------------------------\x1b[0m");
    crate::println!("\x1b[32m  READY\x1b[90m  TYPE \x1b[37mHELP\x1b[90m FOR COMMANDS\x1b[0m");
    crate::println!()
}

#[derive(Clone, Copy)]
enum VtInputState {
    Ground,
    Escape,
    Csi,
    Ss3,
}

struct VtInput {
    state: VtInputState,
    parameter: u16,
}

impl VtInput {
    const fn new() -> Self {
        Self {
            state: VtInputState::Ground,
            parameter: 0,
        }
    }

    fn advance(&mut self, byte: u8) -> Option<Key> {
        match self.state {
            VtInputState::Ground => match byte {
                0x1b => {
                    self.state = VtInputState::Escape;
                    None
                }
                b'\n' => Some(Key::Enter),
                b'\t' => Some(Key::Tab),
                8 | 127 => Some(Key::Backspace),
                3 => Some(Key::Cancel),
                0x20..=0x7e => Some(Key::Character(byte as char)),
                _ => None,
            },
            VtInputState::Escape => {
                self.parameter = 0;
                match byte {
                    b'[' => self.state = VtInputState::Csi,
                    b'O' => self.state = VtInputState::Ss3,
                    _ => self.state = VtInputState::Ground,
                }
                None
            }
            VtInputState::Csi => match byte {
                b'0'..=b'9' => {
                    self.parameter = self
                        .parameter
                        .saturating_mul(10)
                        .saturating_add((byte - b'0') as u16);
                    None
                }
                b';' => None,
                _ => {
                    self.state = VtInputState::Ground;
                    self.navigation_key(byte)
                }
            },
            VtInputState::Ss3 => {
                self.state = VtInputState::Ground;
                self.navigation_key(byte)
            }
        }
    }

    fn navigation_key(&self, byte: u8) -> Option<Key> {
        match byte {
            b'A' => Some(Key::HistoryPrevious),
            b'B' => Some(Key::HistoryNext),
            b'C' => Some(Key::Right),
            b'D' => Some(Key::Left),
            b'H' => Some(Key::Home),
            b'F' => Some(Key::End),
            b'~' => match self.parameter {
                1 | 7 => Some(Key::Home),
                3 => Some(Key::Delete),
                4 | 8 => Some(Key::End),
                _ => None,
            },
            _ => None,
        }
    }
}

fn wait_for_byte(
    keyboard: &mut crate::keyboard::Keyboard,
    usb_keyboard: &mut Option<crate::usb_keyboard::UsbKeyboard>,
    acpi: Option<&AcpiPlatform>,
) -> u8 {
    loop {
        if let Some(byte) = keyboard.read_byte() {
            return byte;
        }
        if let Some(byte) = usb_keyboard
            .as_mut()
            .and_then(crate::usb_keyboard::UsbKeyboard::read_byte)
        {
            return byte;
        }
        if let Some(byte) = crate::console::read_byte() {
            return byte;
        }
        if acpi.is_some_and(crate::power::power_button_pressed) {
            crate::power::shutdown(acpi)
        }
        crate::arch::halt()
    }
}

struct KernelExecutor {
    boot_method: BootMethod,
    memory_regions: &'static [synos_boot_protocol::MemoryRegion],
    memory_region_count: usize,
    memory_total_bytes: u64,
    memory_available_bytes: u64,
    memory_used_bytes: u64,
    scheduler_clock: u64,
    acpi_ready: bool,
    generation: u64,
    completion: Option<(u64, Result<StructuredOutput, Status>)>,
    reboot_requested: bool,
    shutdown_requested: bool,
    monitor: MonitorState,
    scheduler: &'static Scheduler,
    dlm: &'static DistributedLockManager<DEFAULT_LOCK_CAPACITY>,
}

impl KernelExecutor {
    fn new(
        boot_info: &'static BootInfo,
        scheduler: &'static Scheduler,
        dlm: &'static DistributedLockManager<DEFAULT_LOCK_CAPACITY>,
        _node_fences: &'static NodeFenceTable<DEFAULT_NODE_FENCE_CAPACITY>,
        scheduler_clock: u64,
        acpi_ready: bool,
    ) -> Self {
        let memory_total_bytes = boot_info
            .regions()
            .iter()
            .fold(0u64, |total, region| total.saturating_add(region.length));
        let memory_available_bytes = boot_info
            .regions()
            .iter()
            .filter(|region| region.kind == MemoryKind::Usable)
            .fold(0u64, |total, region| total.saturating_add(region.length));

        Self {
            boot_method: boot_info.method,
            memory_regions: boot_info.regions(),
            memory_region_count: boot_info.memory_region_count,
            memory_total_bytes,
            memory_available_bytes,
            memory_used_bytes: memory_total_bytes.saturating_sub(memory_available_bytes),
            scheduler_clock,
            acpi_ready,
            generation: 0,
            completion: None,
            reboot_requested: false,
            shutdown_requested: false,
            monitor: MonitorState::new(),
            scheduler,
            dlm,
        }
    }

    fn execute(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        match command.route.raw() {
            HELP_ROUTE => self.help(),
            SHOW_SYSTEM_ROUTE => self.show_system(),
            REBOOT_ROUTE => self.request_reboot(),
            SHUTDOWN_ROUTE => self.request_shutdown(),
            MONITOR_ROUTE => self.monitor_view(),
            SHOW_PROCESSES_ROUTE => self.show_processes(),
            TOP_CPU_ROUTE => self.top_cpu(),
            SHOW_MEMORY_ROUTE => self.show_memory(),
            SHOW_DSM_ROUTE => self.show_dsm(),
            _ => Err(Status::NOT_FOUND),
        }
    }

    fn help(&self) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(
            &mut output,
            "commands",
            "HELP, SHOW SYSTEM, REBOOT, SHUTDOWN, MONITOR, SHOW PROCESSES, TOP CPU, SHOW MEMORY, SHOW DSM; unique command prefixes accepted",
        )?;
        Ok(output)
    }

    fn show_system(&self) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "name", "SynOS")?;
        insert_text(&mut output, "architecture", architecture())?;
        insert_text(
            &mut output,
            "boot-method",
            match self.boot_method {
                BootMethod::Bios => "BIOS",
                BootMethod::Uefi => "UEFI",
            },
        )?;
        insert(
            &mut output,
            "memory-regions",
            OutputValue::Unsigned(self.memory_region_count as u64),
        )?;
        insert(
            &mut output,
            "scheduler-clock",
            OutputValue::Unsigned(self.scheduler_clock),
        )?;
        insert_text(
            &mut output,
            "acpi",
            if self.acpi_ready { "ready" } else { "unavailable" },
        )?;
        insert_text(&mut output, "shell", "ready")?;
        insert_text(&mut output, "monitor", "active")?;
        Ok(output)
    }

    fn print_system(&self) {
        crate::println!("name=SynOS");
        crate::println!("architecture={}", architecture());
        crate::println!(
            "boot-method={}",
            match self.boot_method {
                BootMethod::Bios => "BIOS",
                BootMethod::Uefi => "UEFI",
            }
        );
        crate::println!("memory-regions={}", self.memory_region_count);
        crate::println!("scheduler-clock={}", self.scheduler_clock);
        crate::println!("acpi={}", if self.acpi_ready { "ready" } else { "unavailable" });
        crate::println!("shell=ready");
        crate::println!("monitor=active")
    }

    fn request_reboot(&mut self) -> Result<StructuredOutput, Status> {
        self.reboot_requested = true;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "action", "rebooting")?;
        Ok(output)
    }

    fn take_reboot_requested(&mut self) -> bool {
        core::mem::take(&mut self.reboot_requested)
    }

    fn request_shutdown(&mut self) -> Result<StructuredOutput, Status> {
        self.shutdown_requested = true;
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "action", "shutting-down")?;
        Ok(output)
    }

    fn take_shutdown_requested(&mut self) -> bool {
        core::mem::take(&mut self.shutdown_requested)
    }

    fn monitor_view(&mut self) -> Result<StructuredOutput, Status> {
        self.monitor.update();
        let view = self.monitor.current_view();
        let view_str = match view {
            MonitorView::Processes => "processes",
            MonitorView::TopCpu => "top-cpu",
            MonitorView::Dsm => "dsm",
            MonitorView::Memory => "memory",
        };
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "view", view_str)?;
        Ok(output)
    }

    fn show_processes(&self) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "view", "processes")?;
        let processes = MonitorState::get_processes(self.scheduler);
        crate::println!("\x1b[1;36m=== PROCESSES ===\x1b[0m");
        crate::println!("THREAD       STATE      SWITCHES SPACE     POLICY     ");
        let mut idx: u64 = 0;
        for proc in processes.iter() {
            if let Some(p) = proc {
                let state_str = match p.state {
                    crate::task::ThreadState::Vacant => "VACANT",
                    crate::task::ThreadState::Ready => "READY",
                    crate::task::ThreadState::Running => "RUNNING",
                    crate::task::ThreadState::Blocked => "BLOCKED",
                    crate::task::ThreadState::Sleeping => "SLEEPING",
                };
                let policy_str = match p.policy {
                    crate::task::SchedulingPolicy::Cooperative => "COOP",
                    crate::task::SchedulingPolicy::Realtime { priority: _, .. } => "RT",
                };
                crate::println!("{} {:<10} {:<8} {:<8} {:<10}",
                    p.thread_id.raw(),
                    state_str,
                    p.switches,
                    p.address_space.raw(),
                    policy_str
                );
                idx += 1;
            }
        }
        crate::println!("Total: {} processes", idx);
        Ok(output)
    }

    fn top_cpu(&mut self) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "view", "cpu")?;
        let top = MonitorState::get_top_cpu(self.scheduler, &mut self.monitor.cpu_history);
        crate::println!("\x1b[1;32m=== TOP CPU ===\x1b[0m");
        crate::println!("THREAD       SWITCHES     UTIL%    ");
        let mut idx: u64 = 0;
        for cpu in top.iter() {
            crate::println!("{} {:<10} {}%",
                cpu.thread_id.raw(),
                cpu.switches,
                cpu.util_percent
            );
            idx += 1;
        }
        crate::println!("Total: {} threads", idx);
        Ok(output)
    }

    fn show_memory(&self) -> Result<StructuredOutput, Status> {
        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "view", "memory")?;
        insert(
            &mut output,
            "total-bytes",
            OutputValue::Unsigned(self.memory_total_bytes),
        )?;
        insert(
            &mut output,
            "available-bytes",
            OutputValue::Unsigned(self.memory_available_bytes),
        )?;
        insert(
            &mut output,
            "used-bytes",
            OutputValue::Unsigned(self.memory_used_bytes),
        )?;
        insert(
            &mut output,
            "memory-regions",
            OutputValue::Unsigned(self.memory_region_count as u64),
        )?;
        crate::println!("\x1b[1;34m=== MEMORY ===\x1b[0m");
        crate::println!("  Total:     {}", memory_size(self.memory_total_bytes));
        crate::println!(
            "  Used:      {}  ({:>4}.{}%)",
            memory_size(self.memory_used_bytes),
            memory_percent(self.memory_used_bytes, self.memory_total_bytes) / 10,
            memory_percent(self.memory_used_bytes, self.memory_total_bytes) % 10,
        );
        crate::println!(
            "  Available: {}  ({:>4}.{}%)",
            memory_size(self.memory_available_bytes),
            memory_percent(self.memory_available_bytes, self.memory_total_bytes) / 10,
            memory_percent(self.memory_available_bytes, self.memory_total_bytes) % 10,
        );
        crate::println!("  Regions:   {}", self.memory_region_count);
        crate::println!();
        for (index, region) in self.memory_regions.iter().enumerate() {
            let available_bytes = region_available_bytes(region.kind, region.length);
            let used_bytes = region_used_bytes(region.kind, region.length);
            crate::println!(
                "  Region {}: {}",
                index,
                memory_kind_name(region.kind),
            );
            crate::println!("    Used:      {}", memory_size(used_bytes));
            crate::println!("    Available: {}", memory_size(available_bytes));
        }
        Ok(output)
    }

    fn show_dsm(&self) -> Result<StructuredOutput, Status> {
        let locks = MonitorState::get_lock_contentions(self.dlm);

        let mut active_locks = 0u64;
        let mut granted_locks = 0u64;
        let mut queued_locks = 0u64;
        for lock in locks.iter() {
            if lock.resource_id.is_some() {
                active_locks += 1;
                granted_locks += lock.granted as u64;
                queued_locks += lock.queued as u64;
            }
        }

        let mut output = StructuredOutput::new(Status::NORMAL);
        insert_text(&mut output, "view", "dsm")?;
        insert(&mut output, "active-locks", OutputValue::Unsigned(active_locks))?;
        insert(&mut output, "granted-locks", OutputValue::Unsigned(granted_locks))?;
        insert(&mut output, "queued-locks", OutputValue::Unsigned(queued_locks))?;

        crate::println!("\x1b[1;33m=== DISTRIBUTED SHARED MEMORY (DSM) LOCKS ===\x1b[0m");
        crate::println!("Active locks: {} / {}", active_locks, MAX_LOCKS);
        crate::println!("Granted:      {}", granted_locks);
        crate::println!("Queued:       {}", queued_locks);

        if active_locks == 0 {
            crate::println!("No active DSM locks.");
            return Ok(output)
        }

        crate::println!();
        crate::println!("RESOURCE   STATE    QUEUED  OWNER NODE");
        for lock in locks.iter() {
            let Some(resource_id) = lock.resource_id else { continue };
            let state = if lock.granted != 0 { "GRANTED" } else { "WAITING" };
            crate::println!("{:<10} {:<8} {:<7} {}",
                resource_id.raw(),
                state,
                lock.queued,
                lock.owner_node
            );
        }
        Ok(output)
    }
}

impl CommandExecutor for KernelExecutor {
    fn submit(
        &mut self,
        command: CommandCall,
        _pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        if self.completion.is_some() {
            return Err(Error::AlreadyRunning);
        }
        self.generation = self.generation.wrapping_add(1).max(1);
        let token = ExecutionToken::new(self.generation).ok_or(Error::InvalidHandle)?;
        let completion = self.execute(command);
        self.completion = Some((token.raw(), completion));
        Ok(token)
    }

    fn poll(&mut self, token: ExecutionToken) -> Option<Result<StructuredOutput, Status>> {
        let (raw, completion) = self.completion.take()?;
        if raw == token.raw() {
            Some(completion)
        } else {
            self.completion = Some((raw, completion));
            Some(Err(Status::NOT_FOUND))
        }
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error> {
        let Some((raw, _)) = self.completion.as_ref() else {
            return Err(Error::InvalidHandle);
        };
        if *raw != token.raw() {
            return Err(Error::InvalidHandle);
        }
        self.completion = None;
        Ok(())
    }
}

fn insert_text(output: &mut StructuredOutput, name: &str, value: &str) -> Result<(), Status> {
    let text = OutputText::new(value).map_err(|_| Status::NO_SPACE)?;
    insert(output, name, OutputValue::Text(text))
}

fn insert(output: &mut StructuredOutput, name: &str, value: OutputValue) -> Result<(), Status> {
    output.insert(name, value).map_err(|_| Status::NO_SPACE)
}

fn memory_size(bytes: u64) -> MemorySize {
    let (divisor, unit) = if bytes >= BYTES_PER_GIB {
        (BYTES_PER_GIB, "GiB")
    } else if bytes >= BYTES_PER_MIB {
        (BYTES_PER_MIB, "MiB")
    } else if bytes >= BYTES_PER_KIB {
        (BYTES_PER_KIB, "KiB")
    } else {
        (1, "bytes")
    };
    let whole = bytes / divisor;
    let tenth = bytes % divisor * 10 + divisor / 2;
    if tenth >= divisor {
        MemorySize(whole + 1, 0, bytes, unit)
    } else {
        MemorySize(whole, tenth / divisor, bytes, unit)
    }
}

fn memory_percent(value: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        value.saturating_mul(1000) / total
    }
}

struct MemorySize(u64, u64, u64, &'static str);

impl core::fmt::Display for MemorySize {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.3 == "bytes" {
            write!(formatter, "{} bytes", self.2)
        } else {
            write!(formatter, "{}.{} {} ({} bytes)", self.0, self.1, self.3, self.2)
        }
    }
}

const BYTES_PER_KIB: u64 = 1024;
const BYTES_PER_MIB: u64 = 1024 * 1024;
const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;

fn region_available_bytes(kind: MemoryKind, length: u64) -> u64 {
    if kind == MemoryKind::Usable { length } else { 0 }
}

fn region_used_bytes(kind: MemoryKind, length: u64) -> u64 {
    if kind == MemoryKind::Usable { 0 } else { length }
}

const fn memory_kind_name(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Usable => "USABLE",
        MemoryKind::Reserved => "RESERVED",
        MemoryKind::AcpiReclaimable => "ACPI RECLAIMABLE",
        MemoryKind::AcpiNonVolatile => "ACPI NONVOLATILE",
        MemoryKind::Bootloader => "BOOTLOADER",
        MemoryKind::Kernel => "KERNEL",
        MemoryKind::Framebuffer => "FRAMEBUFFER",
    }
}

const fn architecture() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "aarch64"
    }
    #[cfg(target_arch = "riscv64")]
    {
        "riscv64"
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "riscv64"
    )))]
    {
        "unknown"
    }
}
